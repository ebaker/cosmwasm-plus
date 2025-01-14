use std::collections::HashMap;
use std::fmt;
use std::ops::Deref;

use cosmwasm_std::{
    Addr, Api, BankMsg, Binary, BlockInfo, Coin, ContractInfo, ContractResult, Deps, DepsMut, Env,
    Event, MessageInfo, Order, Querier, QuerierWrapper, Reply, ReplyOn, Response, Storage, SubMsg,
    SubMsgExecutionResponse, WasmMsg, WasmQuery,
};
use cosmwasm_storage::{prefixed, prefixed_read, PrefixedStorage, ReadonlyPrefixedStorage};
use prost::Message;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cw_storage_plus::Map;

use crate::app::{Router, RouterQuerier};
use crate::contracts::Contract;
use crate::executor::AppResponse;
use crate::transactions::transactional;

// Contract state is kept in Storage, separate from the contracts themselves
const CONTRACTS: Map<&Addr, ContractData> = Map::new("contracts");

pub const NAMESPACE_WASM: &[u8] = b"wasm";

/// Contract Data includes information about contract, equivalent of `ContractInfo` in wasmd
/// interface.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct ContractData {
    /// Identifier of stored contract code
    pub code_id: usize,
    /// Address of account who initially instantiated the contract
    pub creator: Addr,
    /// Optional address of account who can execute migrations
    pub admin: Option<Addr>,
    /// Metadata passed while contract instantiation
    pub label: String,
    /// Blockchain height in the moment of instantiating the contract
    pub created: u64,
}

pub trait Wasm<C>
where
    C: Clone + fmt::Debug + PartialEq + JsonSchema,
{
    /// Handles all WasmQuery requests
    fn query(
        &self,
        storage: &dyn Storage,
        querier: &dyn Querier,
        block: &BlockInfo,
        request: WasmQuery,
    ) -> Result<Binary, String>;

    /// Handles all WasmMsg messages
    fn execute(
        &self,
        storage: &mut dyn Storage,
        router: &Router<C>,
        block: &BlockInfo,
        sender: Addr,
        msg: WasmMsg,
    ) -> Result<AppResponse, String>;

    // Add a new contract. Must be done on the base object, when no contracts running
    fn store_code(&mut self, code: Box<dyn Contract<C>>) -> usize;

    // Helper for querying for specific contract data
    fn contract_data(&self, storage: &dyn Storage, address: &Addr) -> Result<ContractData, String>;

    /// Admin interface, cannot be called via CosmosMsg
    fn sudo(
        &self,
        contract_addr: Addr,
        storage: &mut dyn Storage,
        router: &Router<C>,
        block: &BlockInfo,
        msg: Vec<u8>,
    ) -> Result<AppResponse, String>;
}

pub struct WasmKeeper<C>
where
    C: Clone + fmt::Debug + PartialEq + JsonSchema,
{
    /// code is in-memory lookup that stands in for wasm code
    /// this can only be edited on the WasmRouter, and just read in caches
    codes: HashMap<usize, Box<dyn Contract<C>>>,

    // WasmConst
    api: Box<dyn Api>,
}

impl<C> Wasm<C> for WasmKeeper<C>
where
    C: Clone + fmt::Debug + PartialEq + JsonSchema,
{
    fn query(
        &self,
        storage: &dyn Storage,
        querier: &dyn Querier,
        block: &BlockInfo,
        request: WasmQuery,
    ) -> Result<Binary, String> {
        match request {
            WasmQuery::Smart { contract_addr, msg } => {
                let addr = self
                    .api
                    .addr_validate(&contract_addr)
                    .map_err(|e| e.to_string())?;
                self.query_smart(addr, storage, querier, block, msg.into())
            }
            WasmQuery::Raw { contract_addr, key } => {
                let addr = self
                    .api
                    .addr_validate(&contract_addr)
                    .map_err(|e| e.to_string())?;
                Ok(self.query_raw(addr, storage, &key))
            }
            q => panic!("Unsupported wasm query: {:?}", q),
        }
    }

    fn execute(
        &self,
        storage: &mut dyn Storage,
        router: &Router<C>,
        block: &BlockInfo,
        sender: Addr,
        msg: WasmMsg,
    ) -> Result<AppResponse, String> {
        let (resender, res) = self.execute_wasm(storage, router, block, sender, msg)?;
        self.process_response(router, storage, block, resender, res, false)
    }

    fn store_code(&mut self, code: Box<dyn Contract<C>>) -> usize {
        let idx = self.codes.len() + 1;
        self.codes.insert(idx, code);
        idx
    }

    fn contract_data(&self, storage: &dyn Storage, address: &Addr) -> Result<ContractData, String> {
        self.load_contract(storage, address)
    }

    fn sudo(
        &self,
        contract_addr: Addr,
        storage: &mut dyn Storage,
        router: &Router<C>,
        block: &BlockInfo,
        msg: Vec<u8>,
    ) -> Result<AppResponse, String> {
        let res = self.call_sudo(contract_addr.clone(), storage, router, block, msg)?;
        self.process_response(router, storage, block, contract_addr, res, false)
    }
}

impl<C> WasmKeeper<C>
where
    C: Clone + fmt::Debug + PartialEq + JsonSchema,
{
    pub fn new(api: Box<dyn Api>) -> Self {
        WasmKeeper {
            codes: HashMap::new(),
            api,
        }
    }

    pub fn query_smart(
        &self,
        address: Addr,
        storage: &dyn Storage,
        querier: &dyn Querier,
        block: &BlockInfo,
        msg: Vec<u8>,
    ) -> Result<Binary, String> {
        self.with_storage_readonly(storage, querier, block, address, |handler, deps, env| {
            handler.query(deps, env, msg)
        })
    }

    pub fn query_raw(&self, address: Addr, storage: &dyn Storage, key: &[u8]) -> Binary {
        let storage = self.contract_storage_readonly(storage, &address);
        let data = storage.get(&key).unwrap_or_default();
        data.into()
    }

    fn send<T: Into<Addr>>(
        &self,
        storage: &mut dyn Storage,
        router: &Router<C>,
        block: &BlockInfo,
        sender: T,
        recipient: String,
        amount: &[Coin],
    ) -> Result<AppResponse, String> {
        if !amount.is_empty() {
            let msg = BankMsg::Send {
                to_address: recipient,
                amount: amount.to_vec(),
            };
            let res = router.execute(storage, block, sender.into(), msg.into())?;
            Ok(res)
        } else {
            Ok(AppResponse::default())
        }
    }

    // this returns the contract address as well, so we can properly resend the data
    fn execute_wasm(
        &self,
        storage: &mut dyn Storage,
        router: &Router<C>,
        block: &BlockInfo,
        sender: Addr,
        wasm_msg: WasmMsg,
    ) -> Result<(Addr, Response<C>), String> {
        match wasm_msg {
            WasmMsg::Execute {
                contract_addr,
                msg,
                funds,
            } => {
                let contract_addr = self
                    .api
                    .addr_validate(&contract_addr)
                    .map_err(|e| e.to_string())?;
                // first move the cash
                self.send(
                    storage,
                    router,
                    block,
                    sender.clone(),
                    contract_addr.clone().into(),
                    &funds,
                )?;

                // then call the contract
                let info = MessageInfo { sender, funds };
                let res = self.call_execute(
                    storage,
                    contract_addr.clone(),
                    router,
                    block,
                    info,
                    msg.to_vec(),
                )?;
                Ok((contract_addr, res))
            }
            WasmMsg::Instantiate {
                admin,
                code_id,
                msg,
                funds,
                label,
            } => {
                let contract_addr = self.register_contract(
                    storage,
                    code_id as usize,
                    sender.clone(),
                    admin.map(Addr::unchecked),
                    label,
                    block.height,
                )?;
                // move the cash
                self.send(
                    storage,
                    router,
                    block,
                    sender.clone(),
                    contract_addr.clone().into(),
                    &funds,
                )?;

                // then call the contract
                let info = MessageInfo { sender, funds };
                let mut res = self.call_instantiate(
                    contract_addr.clone(),
                    storage,
                    router,
                    block,
                    info,
                    msg.to_vec(),
                )?;
                init_response(&mut res, &contract_addr);
                Ok((contract_addr, res))
            }
            WasmMsg::Migrate {
                contract_addr,
                new_code_id,
                msg,
            } => {
                let contract_addr = self
                    .api
                    .addr_validate(&contract_addr)
                    .map_err(|e| e.to_string())?;

                // check admin status and update the stored code_id
                let new_code_id = new_code_id as usize;
                if !self.codes.contains_key(&new_code_id) {
                    return Err("Cannot migrate contract to unregistered code id".to_string());
                }
                let mut data = self.load_contract(storage, &contract_addr)?;
                if data.admin != Some(sender) {
                    return Err(format!(
                        "Only admin can migrate contract: {:?}",
                        &data.admin
                    ));
                }
                data.code_id = new_code_id;
                self.save_contract(storage, &contract_addr, &data)?;

                // then call migrate
                let res =
                    self.call_migrate(contract_addr.clone(), storage, router, block, msg.to_vec())?;
                Ok((contract_addr, res))
            }
            m => panic!("Unsupported wasm message: {:?}", m),
        }
    }

    /// This will execute the given messages, making all changes to the local cache.
    /// This *will* write some data to the cache if the message fails half-way through.
    /// All sequential calls to RouterCache will be one atomic unit (all commit or all fail).
    ///
    /// For normal use cases, you can use Router::execute() or Router::execute_multi().
    /// This is designed to be handled internally as part of larger process flows.
    fn execute_submsg(
        &self,
        router: &Router<C>,
        storage: &mut dyn Storage,
        block: &BlockInfo,
        contract: Addr,
        msg: SubMsg<C>,
    ) -> Result<AppResponse, String> {
        let SubMsg {
            msg, id, reply_on, ..
        } = msg;

        // execute in cache
        let res = transactional(storage, |write_cache, _| {
            router.execute(write_cache, block, contract.clone(), msg)
        });

        // call reply if meaningful
        if let Ok(r) = res {
            if matches!(reply_on, ReplyOn::Always | ReplyOn::Success) {
                let mut orig = r.clone();
                let reply = Reply {
                    id,
                    result: ContractResult::Ok(SubMsgExecutionResponse {
                        events: r.events,
                        data: r.data,
                    }),
                };
                // do reply and combine it with the original response
                let res2 = self._reply(router, storage, block, contract, reply)?;
                // override data if set
                if let Some(data) = res2.data {
                    orig.data = Some(data);
                }
                // append the events
                orig.events.extend_from_slice(&res2.events);
                Ok(orig)
            } else {
                Ok(r)
            }
        } else if let Err(e) = res {
            if matches!(reply_on, ReplyOn::Always | ReplyOn::Error) {
                let reply = Reply {
                    id,
                    result: ContractResult::Err(e),
                };
                self._reply(router, storage, block, contract, reply)
            } else {
                Err(e)
            }
        } else {
            res
        }
    }

    fn _reply(
        &self,
        router: &Router<C>,
        storage: &mut dyn Storage,
        block: &BlockInfo,
        contract: Addr,
        reply: Reply,
    ) -> Result<AppResponse, String> {
        let res = self.call_reply(contract.clone(), storage, router, block, reply)?;
        // TODO: process result better, combine events / data from parent
        self.process_response(router, storage, block, contract, res, true)
    }

    fn process_response(
        &self,
        router: &Router<C>,
        storage: &mut dyn Storage,
        block: &BlockInfo,
        contract: Addr,
        response: Response<C>,
        ignore_attributes: bool,
    ) -> Result<AppResponse, String> {
        // These need to get `wasm-` prefix to match the wasmd semantics (custom wasm messages cannot
        // fake system level event types, like transfer from the bank module)
        let mut events: Vec<_> = response
            .events
            .into_iter()
            .map(|mut ev| {
                ev.ty = format!("wasm-{}", ev.ty);
                ev
            })
            .collect();
        // hmmm... we don't need this for reply, right?
        if !ignore_attributes {
            // turn attributes into event and place it first
            let mut wasm_event = Event::new("wasm").add_attribute("contract_address", &contract);
            wasm_event
                .attributes
                .extend_from_slice(&response.attributes);
            events.insert(0, wasm_event);
        }

        // recurse in all messages
        for resend in response.messages {
            let subres = self.execute_submsg(router, storage, block, contract.clone(), resend)?;
            events.extend_from_slice(&subres.events);
        }
        Ok(AppResponse {
            events,
            data: response.data,
        })
    }

    /// This just creates an address and empty storage instance, returning the new address
    /// You must call init after this to set up the contract properly.
    /// These are separated into two steps to have cleaner return values.
    pub fn register_contract(
        &self,
        storage: &mut dyn Storage,
        code_id: usize,
        creator: Addr,
        admin: impl Into<Option<Addr>>,
        label: String,
        created: u64,
    ) -> Result<Addr, String> {
        if !self.codes.contains_key(&code_id) {
            return Err("Cannot init contract with unregistered code id".to_string());
        }

        let addr = self.next_address(&prefixed_read(storage, NAMESPACE_WASM));

        let info = ContractData {
            code_id,
            creator,
            admin: admin.into(),
            label,
            created,
        };
        self.save_contract(storage, &addr, &info)?;
        Ok(addr)
    }

    pub fn call_execute(
        &self,
        storage: &mut dyn Storage,
        address: Addr,
        router: &Router<C>,
        block: &BlockInfo,
        info: MessageInfo,
        msg: Vec<u8>,
    ) -> Result<Response<C>, String> {
        self.with_storage(storage, router, block, address, |contract, deps, env| {
            contract.execute(deps, env, info, msg)
        })
    }

    pub fn call_instantiate(
        &self,
        address: Addr,
        storage: &mut dyn Storage,
        router: &Router<C>,
        block: &BlockInfo,
        info: MessageInfo,
        msg: Vec<u8>,
    ) -> Result<Response<C>, String> {
        self.with_storage(storage, router, block, address, |contract, deps, env| {
            contract.instantiate(deps, env, info, msg)
        })
    }

    pub fn call_reply(
        &self,
        address: Addr,
        storage: &mut dyn Storage,
        router: &Router<C>,
        block: &BlockInfo,
        reply: Reply,
    ) -> Result<Response<C>, String> {
        self.with_storage(storage, router, block, address, |contract, deps, env| {
            contract.reply(deps, env, reply)
        })
    }

    pub fn call_sudo(
        &self,
        address: Addr,
        storage: &mut dyn Storage,
        router: &Router<C>,
        block: &BlockInfo,
        msg: Vec<u8>,
    ) -> Result<Response<C>, String> {
        self.with_storage(storage, router, block, address, |contract, deps, env| {
            contract.sudo(deps, env, msg)
        })
    }

    pub fn call_migrate(
        &self,
        address: Addr,
        storage: &mut dyn Storage,
        router: &Router<C>,
        block: &BlockInfo,
        msg: Vec<u8>,
    ) -> Result<Response<C>, String> {
        self.with_storage(storage, router, block, address, |contract, deps, env| {
            contract.migrate(deps, env, msg)
        })
    }

    fn get_env<T: Into<Addr>>(&self, address: T, block: &BlockInfo) -> Env {
        Env {
            block: block.clone(),
            contract: ContractInfo {
                address: address.into(),
            },
        }
    }

    fn with_storage_readonly<F, T>(
        &self,
        storage: &dyn Storage,
        querier: &dyn Querier,
        block: &BlockInfo,
        address: Addr,
        action: F,
    ) -> Result<T, String>
    where
        F: FnOnce(&Box<dyn Contract<C>>, Deps, Env) -> Result<T, String>,
    {
        let contract = self.load_contract(storage, &address)?;
        let handler = self
            .codes
            .get(&contract.code_id)
            .ok_or_else(|| "Unregistered code id".to_string())?;
        let storage = self.contract_storage_readonly(storage, &address);
        let env = self.get_env(address, block);

        let deps = Deps {
            storage: storage.as_ref(),
            api: self.api.deref(),
            querier: QuerierWrapper::new(querier),
        };
        action(handler, deps, env)
    }

    fn with_storage<F, T>(
        &self,
        storage: &mut dyn Storage,
        router: &Router<C>,
        block: &BlockInfo,
        address: Addr,
        action: F,
    ) -> Result<T, String>
    where
        F: FnOnce(&Box<dyn Contract<C>>, DepsMut, Env) -> Result<T, String>,
    {
        let contract = self.load_contract(storage, &address)?;
        let handler = self
            .codes
            .get(&contract.code_id)
            .ok_or_else(|| "Unregistered code id".to_string())?;

        // We don't actually need a transaction here, as it is already embedded in a transactional.
        // execute_submsg or App.execute_multi.
        // However, we need to get write and read access to the same storage in two different objects,
        // and this is the only way I know how to do so.
        transactional(storage, |write_cache, read_store| {
            let mut contract_storage = self.contract_storage(write_cache, &address);
            let querier = RouterQuerier::new(router, read_store, block);
            let env = self.get_env(address, block);

            let deps = DepsMut {
                storage: contract_storage.as_mut(),
                api: self.api.deref(),
                querier: QuerierWrapper::new(&querier),
            };
            action(handler, deps, env)
        })
    }

    pub fn load_contract(
        &self,
        storage: &dyn Storage,
        address: &Addr,
    ) -> Result<ContractData, String> {
        CONTRACTS
            .load(&prefixed_read(storage, NAMESPACE_WASM), address)
            .map_err(|e| e.to_string())
    }

    pub fn save_contract(
        &self,
        storage: &mut dyn Storage,
        address: &Addr,
        contract: &ContractData,
    ) -> Result<(), String> {
        CONTRACTS
            .save(&mut prefixed(storage, NAMESPACE_WASM), address, contract)
            .map_err(|e| e.to_string())
    }

    // FIXME: better addr generation
    fn next_address(&self, storage: &dyn Storage) -> Addr {
        // FIXME: quite inefficient if we actually had 100s of contracts
        let count = CONTRACTS
            .range(storage, None, None, Order::Ascending)
            .count();
        // we make this longer so it is not rejected by tests
        Addr::unchecked(format!("Contract #{}", count.to_string()))
    }

    fn contract_namespace(&self, contract: &Addr) -> Vec<u8> {
        let mut name = b"contract_data/".to_vec();
        name.extend_from_slice(contract.as_bytes());
        name
    }

    fn contract_storage<'a>(
        &self,
        storage: &'a mut dyn Storage,
        address: &Addr,
    ) -> Box<dyn Storage + 'a> {
        // We double-namespace this, once from global storage -> wasm_storage
        // then from wasm_storage -> the contracts subspace
        let namespace = self.contract_namespace(address);
        let storage = PrefixedStorage::multilevel(storage, &[NAMESPACE_WASM, &namespace]);
        Box::new(storage)
    }

    // fails RUNTIME if you try to write. please don't
    fn contract_storage_readonly<'a>(
        &self,
        storage: &'a dyn Storage,
        address: &Addr,
    ) -> Box<dyn Storage + 'a> {
        // We double-namespace this, once from global storage -> wasm_storage
        // then from wasm_storage -> the contracts subspace
        let namespace = self.contract_namespace(address);
        let storage = ReadonlyPrefixedStorage::multilevel(storage, &[NAMESPACE_WASM, &namespace]);
        Box::new(storage)
    }
}

#[derive(Clone, PartialEq, Message)]
pub struct InstantiateData {
    #[prost(string, tag = "1")]
    pub address: ::prost::alloc::string::String,
    /// Unique ID number for this person.
    #[prost(bytes, tag = "2")]
    pub data: ::prost::alloc::vec::Vec<u8>,
}

fn init_response<C>(res: &mut Response<C>, contact_address: &Addr)
where
    C: Clone + fmt::Debug + PartialEq + JsonSchema,
{
    let data = res.data.clone().unwrap_or_default().to_vec();
    let init_data = InstantiateData {
        address: contact_address.into(),
        data,
    };
    let mut new_data = Vec::<u8>::with_capacity(init_data.encoded_len());
    // the data must encode successfully
    init_data.encode(&mut new_data).unwrap();
    res.data = Some(new_data.into());
}

// this parses the result from a wasm contract init
pub fn parse_contract_addr(data: &Option<Binary>) -> Result<Addr, String> {
    let bin = data
        .as_ref()
        .ok_or_else(|| "No data response".to_string())?
        .to_vec();
    // parse the protobuf struct
    let init_data = InstantiateData::decode(bin.as_slice()).map_err(|e| e.to_string())?;
    if init_data.address.is_empty() {
        return Err("no contract address provided".into());
    }
    Ok(Addr::unchecked(init_data.address))
}

#[cfg(test)]
mod test {
    use cosmwasm_std::testing::{mock_env, mock_info, MockApi, MockQuerier, MockStorage};
    use cosmwasm_std::{coin, from_slice, to_vec, BankMsg, Coin, CosmosMsg, Empty};

    use crate::test_helpers::contracts::{error, payout};
    use crate::transactions::StorageTransaction;
    use crate::BankKeeper;

    use super::*;

    fn mock_keeper() -> WasmKeeper<Empty> {
        let api = Box::new(MockApi::default());
        WasmKeeper::new(api)
    }

    fn mock_router() -> Router<Empty> {
        let api = Box::new(MockApi::default());
        Router::new(api, BankKeeper {})
    }

    #[test]
    fn register_contract() {
        let mut wasm_storage = MockStorage::new();
        let mut keeper = mock_keeper();
        let block = mock_env().block;
        let code_id = keeper.store_code(error::contract());

        transactional(&mut wasm_storage, |cache, _| {
            // cannot register contract with unregistered codeId
            keeper.register_contract(
                cache,
                code_id + 1,
                Addr::unchecked("foobar"),
                Addr::unchecked("admin"),
                "label".to_owned(),
                1000,
            )
        })
        .unwrap_err();

        let contract_addr = transactional(&mut wasm_storage, |cache, _| {
            // we can register a new instance of this code
            keeper.register_contract(
                cache,
                code_id,
                Addr::unchecked("foobar"),
                Addr::unchecked("admin"),
                "label".to_owned(),
                1000,
            )
        })
        .unwrap();

        // verify contract data are as expected
        let contract_data = keeper.load_contract(&wasm_storage, &contract_addr).unwrap();

        assert_eq!(
            contract_data,
            ContractData {
                code_id,
                creator: Addr::unchecked("foobar"),
                admin: Some(Addr::unchecked("admin")),
                label: "label".to_owned(),
                created: 1000,
            }
        );

        let err = transactional(&mut wasm_storage, |cache, _| {
            // now, we call this contract and see the error message from the contract
            let info = mock_info("foobar", &[]);
            keeper.call_instantiate(
                contract_addr.clone(),
                cache,
                &mock_router(),
                &block,
                info,
                b"{}".to_vec(),
            )
        })
        .unwrap_err();

        // StdError from contract_error auto-converted to string
        assert_eq!(err, "Generic error: Init failed");

        let err = transactional(&mut wasm_storage, |cache, _| {
            // and the error for calling an unregistered contract
            let info = mock_info("foobar", &[]);
            keeper.call_instantiate(
                Addr::unchecked("unregistered"),
                cache,
                &mock_router(),
                &block,
                info,
                b"{}".to_vec(),
            )
        })
        .unwrap_err();

        // Default error message from router when not found
        assert_eq!(err, "cw_multi_test::wasm::ContractData not found");
    }

    #[test]
    fn contract_send_coins() {
        let mut keeper = mock_keeper();
        let block = mock_env().block;
        let code_id = keeper.store_code(payout::contract());

        let mut wasm_storage = MockStorage::new();
        let mut cache = StorageTransaction::new(&wasm_storage);

        let contract_addr = keeper
            .register_contract(
                &mut cache,
                code_id,
                Addr::unchecked("foobar"),
                None,
                "label".to_owned(),
                1000,
            )
            .unwrap();

        let payout = coin(100, "TGD");

        // init the contract
        let info = mock_info("foobar", &[]);
        let init_msg = to_vec(&payout::InstantiateMessage {
            payout: payout.clone(),
        })
        .unwrap();
        let res = keeper
            .call_instantiate(
                contract_addr.clone(),
                &mut cache,
                &mock_router(),
                &block,
                info,
                init_msg,
            )
            .unwrap();
        assert_eq!(0, res.messages.len());

        // execute the contract
        let info = mock_info("foobar", &[]);
        let res = keeper
            .call_execute(
                &mut cache,
                contract_addr.clone(),
                &mock_router(),
                &block,
                info,
                b"{}".to_vec(),
            )
            .unwrap();
        assert_eq!(1, res.messages.len());
        match &res.messages[0].msg {
            CosmosMsg::Bank(BankMsg::Send { to_address, amount }) => {
                assert_eq!(to_address.as_str(), "foobar");
                assert_eq!(amount.as_slice(), &[payout.clone()]);
            }
            m => panic!("Unexpected message {:?}", m),
        }

        // and flush before query
        cache.prepare().commit(&mut wasm_storage);

        // query the contract
        let query = to_vec(&payout::QueryMsg::Payout {}).unwrap();
        let querier: MockQuerier<Empty> = MockQuerier::new(&[]);
        let data = keeper
            .query_smart(contract_addr, &wasm_storage, &querier, &block, query)
            .unwrap();
        let res: payout::InstantiateMessage = from_slice(&data).unwrap();
        assert_eq!(res.payout, payout);
    }

    fn assert_payout(
        router: &WasmKeeper<Empty>,
        storage: &mut dyn Storage,
        contract_addr: &Addr,
        payout: &Coin,
    ) {
        let info = mock_info("silly", &[]);
        let res = router
            .call_execute(
                storage,
                contract_addr.clone(),
                &mock_router(),
                &mock_env().block,
                info,
                b"{}".to_vec(),
            )
            .unwrap();
        assert_eq!(1, res.messages.len());
        match &res.messages[0].msg {
            CosmosMsg::Bank(BankMsg::Send { to_address, amount }) => {
                assert_eq!(to_address.as_str(), "silly");
                assert_eq!(amount.as_slice(), &[payout.clone()]);
            }
            m => panic!("Unexpected message {:?}", m),
        }
    }

    fn assert_no_contract(storage: &dyn Storage, contract_addr: &Addr) {
        let contract = CONTRACTS.may_load(storage, contract_addr).unwrap();
        assert!(contract.is_none(), "{:?}", contract_addr);
    }

    #[test]
    fn multi_level_wasm_cache() {
        let mut keeper = mock_keeper();
        let block = mock_env().block;
        let code_id = keeper.store_code(payout::contract());

        let mut wasm_storage = MockStorage::new();

        let payout1 = coin(100, "TGD");

        // set contract 1 and commit (on router)
        let contract1 = transactional(&mut wasm_storage, |cache, _| {
            let contract = keeper
                .register_contract(
                    cache,
                    code_id,
                    Addr::unchecked("foobar"),
                    None,
                    "".to_string(),
                    1000,
                )
                .unwrap();
            let info = mock_info("foobar", &[]);
            let init_msg = to_vec(&payout::InstantiateMessage {
                payout: payout1.clone(),
            })
            .unwrap();
            keeper
                .call_instantiate(
                    contract.clone(),
                    cache,
                    &mock_router(),
                    &block,
                    info,
                    init_msg,
                )
                .unwrap();

            Ok(contract)
        })
        .unwrap();

        let payout2 = coin(50, "BTC");
        let payout3 = coin(1234, "ATOM");

        // create a new cache and check we can use contract 1
        let (contract2, contract3) = transactional(&mut wasm_storage, |cache, wasm_reader| {
            assert_payout(&keeper, cache, &contract1, &payout1);

            // create contract 2 and use it
            let contract2 = keeper
                .register_contract(
                    cache,
                    code_id,
                    Addr::unchecked("foobar"),
                    None,
                    "".to_owned(),
                    1000,
                )
                .unwrap();
            let info = mock_info("foobar", &[]);
            let init_msg = to_vec(&payout::InstantiateMessage {
                payout: payout2.clone(),
            })
            .unwrap();
            let _res = keeper
                .call_instantiate(
                    contract2.clone(),
                    cache,
                    &mock_router(),
                    &block,
                    info,
                    init_msg,
                )
                .unwrap();
            assert_payout(&keeper, cache, &contract2, &payout2);

            // create a level2 cache and check we can use contract 1 and contract 2
            let contract3 = transactional(cache, |cache2, read| {
                assert_payout(&keeper, cache2, &contract1, &payout1);
                assert_payout(&keeper, cache2, &contract2, &payout2);

                // create a contract on level 2
                let contract3 = keeper
                    .register_contract(
                        cache2,
                        code_id,
                        Addr::unchecked("foobar"),
                        None,
                        "".to_owned(),
                        1000,
                    )
                    .unwrap();
                let info = mock_info("johnny", &[]);
                let init_msg = to_vec(&payout::InstantiateMessage {
                    payout: payout3.clone(),
                })
                .unwrap();
                let _res = keeper
                    .call_instantiate(
                        contract3.clone(),
                        cache2,
                        &mock_router(),
                        &block,
                        info,
                        init_msg,
                    )
                    .unwrap();
                assert_payout(&keeper, cache2, &contract3, &payout3);

                // ensure first cache still doesn't see this contract
                assert_no_contract(read, &contract3);
                Ok(contract3)
            })
            .unwrap();

            // after applying transaction, all contracts present on cache
            assert_payout(&keeper, cache, &contract1, &payout1);
            assert_payout(&keeper, cache, &contract2, &payout2);
            assert_payout(&keeper, cache, &contract3, &payout3);

            // but not yet the root router
            assert_no_contract(wasm_reader, &contract1);
            assert_no_contract(wasm_reader, &contract2);
            assert_no_contract(wasm_reader, &contract3);

            Ok((contract2, contract3))
        })
        .unwrap();

        // ensure that it is now applied to the router
        assert_payout(&keeper, &mut wasm_storage, &contract1, &payout1);
        assert_payout(&keeper, &mut wasm_storage, &contract2, &payout2);
        assert_payout(&keeper, &mut wasm_storage, &contract3, &payout3);
    }
}

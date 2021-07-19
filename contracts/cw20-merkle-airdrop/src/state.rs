use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_std::Addr;
use cw_storage_plus::{Item, Map, U8Key};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Config {
    pub owner: Addr,
    pub cw20_token_address: Addr,
}

pub const CONFIG_KEY: &str = "config";
pub const CONFIG: Item<Config> = Item::new(CONFIG_KEY);

pub const STAGE_KEY: &str = "stage";
pub const STAGE: Item<u8> = Item::new(STAGE_KEY);

pub const MERKLE_ROOT_PREFIX: &str = "merkle_root";
pub const MERKLE_ROOT: Map<U8Key, String> = Map::new(MERKLE_ROOT_PREFIX);

pub const CLAIM_PREFIX: &str = "claim";
pub const CLAIM: Map<(&Addr, U8Key), bool> = Map::new(CLAIM_PREFIX);
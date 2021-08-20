/* tslint:disable */
/**
 * This file was automatically generated by json-schema-to-typescript.
 * DO NOT MODIFY IT BY HAND. Instead, modify the source JSONSchema file,
 * and run json-schema-to-typescript to regenerate this file.
 */

/**
 * A fixed-point decimal value with 18 fractional digits, i.e. Decimal(1_000_000_000_000_000_000) == 1.0
 *
 * The greatest possible value that can be represented is 340282366920938463463.374607431768211455 (which is (2^128 - 1) / 10^18)
 */
export type Decimal = string;
/**
 * A thin wrapper around u128 that is using strings for JSON encoding/decoding, such that the full u128 range can be used for clients that convert JSON numbers to floats, like JavaScript and jq.
 *
 * # Examples
 *
 * Use `from` to create instances of this and `u128` to get the value out:
 *
 * ``` # use cosmwasm_std::Uint128; let a = Uint128::from(123u128); assert_eq!(a.u128(), 123);
 *
 * let b = Uint128::from(42u64); assert_eq!(b.u128(), 42);
 *
 * let c = Uint128::from(70u32); assert_eq!(c.u128(), 70); ```
 */
export type Uint128 = string;
/**
 * Duration is a delta of time. You can add it to a BlockInfo or Expiration to move that further in the future. Note that an height-based Duration and a time-based Expiration cannot be combined
 */
export type Duration =
  | {
      height: number;
    }
  | {
      time: number;
    };

export interface InstantiateMsg {
  /**
   * decimal places of the derivative token (for UI)
   */
  decimals: number;
  /**
   * this is how much the owner takes as a cut when someone unbonds
   */
  exit_tax: Decimal;
  /**
   * This is the minimum amount we will pull out to reinvest, as well as a minimum that can be unbonded (to avoid needless staking tx)
   */
  min_withdrawal: Uint128;
  /**
   * name of the derivative token
   */
  name: string;
  /**
   * symbol / ticker of the derivative token
   */
  symbol: string;
  /**
   * This is the unbonding period of the native staking module We need this to only allow claims to be redeemed after the money has arrived
   */
  unbonding_period: Duration;
  /**
   * This is the validator that all tokens will be bonded to
   */
  validator: string;
  [k: string]: unknown;
}

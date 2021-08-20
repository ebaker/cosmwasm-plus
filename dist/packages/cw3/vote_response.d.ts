/* tslint:disable */
/**
 * This file was automatically generated by json-schema-to-typescript.
 * DO NOT MODIFY IT BY HAND. Instead, modify the source JSONSchema file,
 * and run json-schema-to-typescript to regenerate this file.
 */

export type Vote = "yes" | "no" | "abstain" | "veto";

export interface VoteResponse {
  vote?: VoteInfo | null;
  [k: string]: unknown;
}
/**
 * Returns the vote (opinion as well as weight counted) as well as the address of the voter who submitted it
 */
export interface VoteInfo {
  vote: Vote;
  voter: string;
  weight: number;
  [k: string]: unknown;
}

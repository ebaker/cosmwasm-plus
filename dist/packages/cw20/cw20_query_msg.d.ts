/* tslint:disable */
/**
 * This file was automatically generated by json-schema-to-typescript.
 * DO NOT MODIFY IT BY HAND. Instead, modify the source JSONSchema file,
 * and run json-schema-to-typescript to regenerate this file.
 */

export type Cw20QueryMsg =
  | {
      balance: {
        address: string;
        [k: string]: unknown;
      };
    }
  | {
      token_info: {
        [k: string]: unknown;
      };
    }
  | {
      allowance: {
        owner: string;
        spender: string;
        [k: string]: unknown;
      };
    }
  | {
      minter: {
        [k: string]: unknown;
      };
    }
  | {
      all_allowances: {
        limit?: number | null;
        owner: string;
        start_after?: string | null;
        [k: string]: unknown;
      };
    }
  | {
      all_accounts: {
        limit?: number | null;
        start_after?: string | null;
        [k: string]: unknown;
      };
    };

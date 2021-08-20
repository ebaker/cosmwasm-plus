/* tslint:disable */
/**
 * This file was automatically generated by json-schema-to-typescript.
 * DO NOT MODIFY IT BY HAND. Instead, modify the source JSONSchema file,
 * and run json-schema-to-typescript to regenerate this file.
 */

export interface NftInfoResponse {
  /**
   * Describes the asset to which this NFT represents
   */
  description: string;
  /**
   * "A URI pointing to a resource with mime type image/* representing the asset to which this NFT represents. Consider making any images at a width between 320 and 1080 pixels and aspect ratio between 1.91:1 and 4:5 inclusive. TODO: Use https://docs.rs/url_serde for type-safety
   */
  image?: string | null;
  /**
   * Identifies the asset to which this NFT represents
   */
  name: string;
  [k: string]: unknown;
}

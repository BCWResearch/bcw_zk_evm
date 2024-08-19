//! This is useful for fetching [ProverInput] per block
use std::fs;

use alloy::rpc::types::{BlockId, BlockNumberOrTag};
use anyhow::Error;
use google_cloud_storage::{
    client::{Client, ClientConfig},
    http::objects::{download::Range, get::GetObjectRequest},
};
use prover::{BlockProverInput, ProverInput};
use rpc::{benchmark_prover_input, retry::build_http_retry_provider, BenchmarkedProverInput};
use serde::Deserialize;
use tracing::info;
use zero_bin_common::block_interval::BlockInterval;

use super::input::BlockSource;

//==============================================================================
// FetchError
//==============================================================================
#[derive(Debug)]
pub enum FetchError {
    RpcFetchError(Error),
    LocalFileErr(Error),
    GcsErr(Error),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self, f)
    }
}

impl std::error::Error for FetchError {}

//=============================================================================
// Fetching
//=============================================================================

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub enum Checkpoint {
    Constant(BlockId),
    BlockNumberNegativeOffset(u64),
}

unsafe impl Send for Checkpoint {}

impl Default for Checkpoint {
    fn default() -> Self {
        Self::BlockNumberNegativeOffset(1)
    }
}

impl Checkpoint {
    pub fn get_checkpoint_from_blocknum(&self, block_number: u64) -> BlockId {
        match self {
            Self::Constant(num @ BlockId::Number(_)) => *num,
            Self::Constant(BlockId::Hash(_)) => {
                unreachable!("Coordinator does not support Hash Block IDs")
            }
            Self::BlockNumberNegativeOffset(offset) => {
                BlockId::Number(BlockNumberOrTag::Number(block_number - *offset))
            }
            #[allow(unreachable_patterns)]
            _ => BlockId::Number(BlockNumberOrTag::Number(block_number - 1)),
        }
    }

    pub fn get_checkpoint_from_interval(&self, block_interval: BlockInterval) -> BlockId {
        match block_interval {
            BlockInterval::FollowFrom {
                start_block,
                block_time: _,
            } => self.get_checkpoint_from_blocknum(start_block),
            BlockInterval::Range(range) => self.get_checkpoint_from_blocknum(range.start),
            BlockInterval::SingleBlockId(BlockId::Number(BlockNumberOrTag::Number(start))) => {
                self.get_checkpoint_from_blocknum(start)
            }
            BlockInterval::SingleBlockId(BlockId::Number(_) | BlockId::Hash(_)) => {
                todo!("Coordinator only supports Numbers, not Tags or Block Hashes")
            }
        }
    }
}

/// Fetches the prover input given the [BlockSource]
pub async fn fetch(source: &BlockSource) -> Result<BenchmarkedProverInput, FetchError> {
    match source {
        // Use ZeroBing's RPC fetch
        BlockSource::Rpc {
            rpc_url,
            block_interval,
            checkpoint,
            backoff,
            max_retries,
            rpc_type,
        } => {
            info!(
                "Requesting from block {} from RPC ({})",
                block_interval, rpc_url
            );

            let provider_url = match url::Url::parse(rpc_url) {
                Ok(url) => url,
                Err(err) => return Err(FetchError::RpcFetchError(err.into())),
            };

            let cached_provider = rpc::provider::CachedProvider::new(build_http_retry_provider(
                provider_url,
                backoff.unwrap_or(0),
                max_retries.unwrap_or(0),
            ));

            let block_iv = match BlockInterval::new(block_interval) {
                Ok(bi) => bi,
                Err(err) => panic!(
                    "Failed to create BlockInterval from {}: {}",
                    block_interval, err
                ),
            };

            let checkpoint_block_id = match checkpoint.unwrap_or_default() {
                Checkpoint::Constant(block_id) => block_id,
                Checkpoint::BlockNumberNegativeOffset(offset) => match &block_iv {
                    BlockInterval::FollowFrom {
                        start_block,
                        block_time: _,
                    } => BlockId::Number(BlockNumberOrTag::Number(start_block - offset)),
                    BlockInterval::Range(range) => {
                        BlockId::Number(BlockNumberOrTag::Number(range.start - offset))
                    }
                    BlockInterval::SingleBlockId(BlockId::Number(BlockNumberOrTag::Number(
                        num,
                    ))) => BlockId::Number(BlockNumberOrTag::Number(num - 1)),
                    BlockInterval::SingleBlockId(_) => {
                        unimplemented!("No support for checkpoints and hash/tags")
                    }
                },
            };

            match benchmark_prover_input(
                &cached_provider,
                block_iv,
                checkpoint_block_id,
                rpc_type.clone().unwrap_or(rpc::RpcType::Jerigon),
            )
            .await
            {
                Ok(input) => Ok(input),
                Err(err) => Err(FetchError::RpcFetchError(err)),
            }
        }
        BlockSource::LocalFile { filepath } => match fs::read_to_string(filepath) {
            Ok(string) => {
                let proverinput = match from_string(&string) {
                    Ok(proverinput) => proverinput,
                    Err(err) => {
                        tracing::error!("Failed to convert file into ProverInput: {}", err);
                        return Err(FetchError::LocalFileErr(err));
                    }
                };

                Ok(BenchmarkedProverInput {
                    proverinput,
                    fetch_times: Vec::new(),
                })
            }
            Err(err) => {
                tracing::error!("Failed to read local file: {}", filepath);
                Err(FetchError::LocalFileErr(err.into()))
            }
        },
        BlockSource::Gcs { filepath, bucket } => {
            let client_config = ClientConfig::default();

            let client = Client::new(client_config);

            let req = GetObjectRequest {
                bucket: bucket.clone(),
                object: filepath.clone(),
                ..GetObjectRequest::default()
            };

            let range = Range::default();

            let string = match client.download_object(&req, &range).await {
                Ok(byte_data) => match String::from_utf8(byte_data) {
                    Ok(string) => string,
                    Err(err) => {
                        tracing::error!(
                            "Failed to convert returned data into utf8 string: {}",
                            err
                        );
                        return Err(FetchError::GcsErr(err.into()));
                    }
                },
                Err(err) => {
                    tracing::error!("Failed to pull witness from GCS: {}", err);
                    return Err(FetchError::GcsErr(err.into()));
                }
            };

            match from_string(&string) {
                Ok(proverinput) => Ok(BenchmarkedProverInput {
                    proverinput,
                    fetch_times: Vec::new(),
                }),
                Err(err) => {
                    tracing::error!("Failed to deserialize string into ProverInput: {}", err);
                    Err(FetchError::GcsErr(err))
                }
            }
        }
    }
}

fn from_string(string: &str) -> Result<ProverInput, Error> {
    let des = &mut serde_json::Deserializer::from_str(string);

    match Vec::<BlockProverInput>::deserialize(des) {
        Ok(blocks) => Ok(ProverInput { blocks }),
        Err(err) => {
            tracing::error!("Failed to deserialize vec of BlockProverInput: {}", err);
            Err(err.into())
        }
    }
}

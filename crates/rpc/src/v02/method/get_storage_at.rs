use crate::context::RpcContext;
use crate::felt::RpcFelt;
use anyhow::Context;
use pathfinder_common::{BlockId, ContractAddress, StorageAddress, StorageValue};
use serde::Deserialize;

#[derive(Deserialize, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GetStorageAtInput {
    pub contract_address: ContractAddress,
    pub key: StorageAddress,
    pub block_id: BlockId,
}

#[serde_with::serde_as]
#[derive(serde::Serialize, Debug)]
pub struct GetStorageOutput(#[serde_as(as = "RpcFelt")] StorageValue);

crate::error::generate_rpc_error_subset!(GetStorageAtError: ContractNotFound, BlockNotFound);

/// Get the value of the storage at the given address and key.
pub async fn get_storage_at(
    context: RpcContext,
    input: GetStorageAtInput,
) -> Result<GetStorageOutput, GetStorageAtError> {
    let storage = context.storage.clone();
    let span = tracing::Span::current();

    let jh = tokio::task::spawn_blocking(move || {
        let _g = span.enter();
        let mut db = storage
            .connection()
            .context("Opening database connection")?;

        let tx = db.transaction().context("Creating database transaction")?;

        if input.block_id.is_pending() {
            if let Some(value) = context
                .pending_data
                .get(&tx)
                .context("Querying pending data")?
                .state_update
                .storage_value(input.contract_address, input.key)
            {
                return Ok(GetStorageOutput(value));
            }
        }

        let block_id = match input.block_id {
            BlockId::Pending => pathfinder_storage::BlockId::Latest,
            other => other.try_into().expect("Only pending cast should fail"),
        };

        // Check for block existence.
        if !tx.block_exists(block_id)? {
            return Err(GetStorageAtError::BlockNotFound);
        }

        let value = tx
            .storage_value(block_id, input.contract_address, input.key)
            .context("Querying storage value")?;

        match value {
            Some(value) => Ok(GetStorageOutput(value)),
            None => {
                if tx.contract_exists(input.contract_address, block_id)? {
                    Ok(GetStorageOutput(StorageValue::ZERO))
                } else {
                    Err(GetStorageAtError::ContractNotFound)
                }
            }
        }
    });

    jh.await.context("Database read panic or shutting down")?
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_matches::assert_matches;
    use serde_json::json;

    use pathfinder_common::{macro_prelude::*, BlockNumber};

    /// # Important
    ///
    /// `BlockId` parsing is tested in [`get_block`][crate::rpc::v02::method::get_block::tests::parsing]
    /// and is not repeated here.
    #[rstest::rstest]
    #[case::positional(json!(["1", "2", "latest"]))]
    #[case::named(json!({"contract_address": "0x1", "key": "0x2", "block_id": "latest"}))]
    fn parsing(#[case] input: serde_json::Value) {
        let expected = GetStorageAtInput {
            contract_address: contract_address!("0x1"),
            key: storage_address!("0x2"),
            block_id: BlockId::Latest,
        };

        let input = serde_json::from_value::<GetStorageAtInput>(input).unwrap();

        assert_eq!(input, expected);
    }

    #[tokio::test]
    async fn pending() {
        let ctx = RpcContext::for_tests_with_pending().await;
        let contract_address = contract_address_bytes!(b"pending contract 1 address");
        let key = storage_address_bytes!(b"pending storage key 0");
        let block_id = BlockId::Pending;

        let result = get_storage_at(
            ctx,
            GetStorageAtInput {
                contract_address,
                key,
                block_id,
            },
        )
        .await
        .unwrap();

        assert_eq!(result.0, storage_value_bytes!(b"pending storage value 0"));
    }

    #[tokio::test]
    async fn pending_falls_back_to_latest() {
        let ctx = RpcContext::for_tests_with_pending().await;
        let contract_address = contract_address_bytes!(b"contract 1");
        let key = storage_address_bytes!(b"storage addr 0");
        let block_id = BlockId::Pending;

        let result = get_storage_at(
            ctx,
            GetStorageAtInput {
                contract_address,
                key,
                block_id,
            },
        )
        .await
        .unwrap();

        assert_eq!(result.0, storage_value_bytes!(b"storage value 2"));
    }

    #[tokio::test]
    async fn pending_deployed_defaults_to_zero() {
        let ctx = RpcContext::for_tests_with_pending().await;
        // Contract is deployed in pending block, but has no storage values set.
        let contract_address = contract_address_bytes!(b"pending contract 0 address");
        let key = storage_address_bytes!(b"non-existent");
        let block_id = BlockId::Pending;

        let result = get_storage_at(
            ctx,
            GetStorageAtInput {
                contract_address,
                key,
                block_id,
            },
        )
        .await
        .unwrap();

        assert_eq!(result.0, StorageValue::ZERO);
    }

    #[tokio::test]
    async fn latest() {
        let ctx = RpcContext::for_tests_with_pending().await;
        let contract_address = contract_address_bytes!(b"contract 1");
        let key = storage_address_bytes!(b"storage addr 0");
        let block_id = BlockId::Latest;

        let result = get_storage_at(
            ctx,
            GetStorageAtInput {
                contract_address,
                key,
                block_id,
            },
        )
        .await
        .unwrap();

        assert_eq!(result.0, storage_value_bytes!(b"storage value 2"));
    }

    #[tokio::test]
    async fn defaults_to_zero() {
        let ctx = RpcContext::for_tests_with_pending().await;
        let contract_address = contract_address_bytes!(b"contract 1");
        let key = storage_address_bytes!(b"non-existent");
        let block_id = BlockId::Latest;

        let result = get_storage_at(
            ctx,
            GetStorageAtInput {
                contract_address,
                key,
                block_id,
            },
        )
        .await
        .unwrap();

        assert_eq!(result.0, StorageValue::ZERO);
    }

    #[tokio::test]
    async fn by_hash() {
        let ctx = RpcContext::for_tests_with_pending().await;
        let contract_address = contract_address_bytes!(b"contract 1");
        let key = storage_address_bytes!(b"storage addr 0");
        let block_id = BlockId::Hash(block_hash_bytes!(b"block 1"));

        let result = get_storage_at(
            ctx,
            GetStorageAtInput {
                contract_address,
                key,
                block_id,
            },
        )
        .await
        .unwrap();

        assert_eq!(result.0, storage_value_bytes!(b"storage value 1"));
    }

    #[tokio::test]
    async fn by_number() {
        let ctx = RpcContext::for_tests_with_pending().await;
        let contract_address = contract_address_bytes!(b"contract 1");
        let key = storage_address_bytes!(b"storage addr 0");
        let block_id = BlockId::Number(BlockNumber::GENESIS + 1);

        let result = get_storage_at(
            ctx,
            GetStorageAtInput {
                contract_address,
                key,
                block_id,
            },
        )
        .await
        .unwrap();

        assert_eq!(result.0, storage_value_bytes!(b"storage value 1"));
    }

    #[tokio::test]
    async fn unknown_contract() {
        let ctx = RpcContext::for_tests_with_pending().await;
        let contract_address = contract_address_bytes!(b"non-existent");
        let key = storage_address_bytes!(b"storage addr 0");
        let block_id = BlockId::Latest;

        let result = get_storage_at(
            ctx,
            GetStorageAtInput {
                contract_address,
                key,
                block_id,
            },
        )
        .await;

        assert_matches!(result, Err(GetStorageAtError::ContractNotFound));
    }

    #[tokio::test]
    async fn contract_is_unknown_before_deployment() {
        let ctx = RpcContext::for_tests_with_pending().await;
        let contract_address = contract_address_bytes!(b"contract 1");
        let key = storage_address_bytes!(b"storage addr 0");
        let block_id = BlockId::Hash(block_hash_bytes!(b"genesis"));

        let result = get_storage_at(
            ctx,
            GetStorageAtInput {
                contract_address,
                key,
                block_id,
            },
        )
        .await;

        assert_matches!(result, Err(GetStorageAtError::ContractNotFound));
    }

    #[tokio::test]
    async fn block_not_found_by_number() {
        let ctx = RpcContext::for_tests_with_pending().await;
        let contract_address = contract_address_bytes!(b"contract 1");
        let key = storage_address_bytes!(b"storage addr 0");
        let block_id = BlockId::Number(BlockNumber::MAX);

        let result = get_storage_at(
            ctx,
            GetStorageAtInput {
                contract_address,
                key,
                block_id,
            },
        )
        .await;

        assert_matches!(result, Err(GetStorageAtError::BlockNotFound));
    }

    #[tokio::test]
    async fn block_not_found_by_hash() {
        let ctx = RpcContext::for_tests_with_pending().await;
        let contract_address = contract_address_bytes!(b"contract 1");
        let key = storage_address_bytes!(b"storage addr 0");
        let block_id = BlockId::Hash(block_hash_bytes!(b"unknown"));

        let result = get_storage_at(
            ctx,
            GetStorageAtInput {
                contract_address,
                key,
                block_id,
            },
        )
        .await;

        assert_matches!(result, Err(GetStorageAtError::BlockNotFound));
    }
}

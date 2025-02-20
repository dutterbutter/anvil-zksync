use crate::node::{InMemoryNode, TransactionResult};
use crate::utils::{internal_error, utc_datetime_from_epoch_ms};
use std::collections::HashMap;
use zksync_types::api::{
    BlockDetails, BlockDetailsBase, BlockStatus, BridgeAddresses, TransactionDetails,
    TransactionStatus, TransactionVariant,
};
use zksync_types::fee::Fee;
use zksync_types::h256_to_u256;
use zksync_types::transaction_request::CallRequest;
use zksync_types::utils::storage_key_for_standard_token_balance;
use zksync_types::{
    AccountTreeId, Address, ExecuteTransactionCommon, L1BatchNumber, L2BlockNumber,
    ProtocolVersionId, Transaction, H160, H256, L2_BASE_TOKEN_ADDRESS, U256,
};
use zksync_web3_decl::error::Web3Error;

impl InMemoryNode {
    pub async fn estimate_fee_impl(&self, req: CallRequest) -> Result<Fee, Web3Error> {
        // TODO: Burn with fire
        let time = self.time.lock();
        self.read_inner()?.estimate_gas_impl(&time, req)
    }

    pub async fn get_raw_block_transactions_impl(
        &self,
        block_number: L2BlockNumber,
    ) -> Result<Vec<Transaction>, Web3Error> {
        let reader = self.read_inner()?;

        let maybe_transactions = reader
            .block_hashes
            .get(&(block_number.0 as u64))
            .and_then(|hash| reader.blocks.get(hash))
            .map(|block| {
                block
                    .transactions
                    .iter()
                    .map(|tx| match tx {
                        TransactionVariant::Full(tx) => &tx.hash,
                        TransactionVariant::Hash(hash) => hash,
                    })
                    .flat_map(|tx_hash| {
                        reader
                            .tx_results
                            .get(tx_hash)
                            .map(|TransactionResult { info, .. }| Transaction {
                                common_data: ExecuteTransactionCommon::L2(
                                    info.tx.common_data.clone(),
                                ),
                                execute: info.tx.execute.clone(),
                                received_timestamp_ms: info.tx.received_timestamp_ms,
                                raw_bytes: info.tx.raw_bytes.clone(),
                            })
                    })
                    .collect()
            });

        let transactions = match maybe_transactions {
            Some(txns) => Ok(txns),
            None => {
                let fork_storage_read = reader
                    .fork_storage
                    .inner
                    .read()
                    .expect("failed reading fork storage");

                match fork_storage_read.fork.as_ref() {
                    Some(fork) => fork
                        .fork_source
                        .get_raw_block_transactions(block_number)
                        .map_err(|e| internal_error("get_raw_block_transactions", e)),
                    None => Ok(vec![]),
                }
            }
        }?;

        Ok(transactions)
    }

    pub async fn get_bridge_contracts_impl(&self) -> Result<BridgeAddresses, Web3Error> {
        let reader = self.read_inner()?;

        let result = match reader
            .fork_storage
            .inner
            .read()
            .expect("failed reading fork storage")
            .fork
            .as_ref()
        {
            Some(fork) => fork.fork_source.get_bridge_contracts().map_err(|err| {
                tracing::error!("failed fetching bridge contracts from the fork: {:?}", err);
                Web3Error::InternalError(anyhow::Error::msg(format!(
                    "failed fetching bridge contracts from the fork: {:?}",
                    err
                )))
            })?,
            None => BridgeAddresses {
                l1_shared_default_bridge: Default::default(),
                l2_shared_default_bridge: Default::default(),
                l1_erc20_default_bridge: Default::default(),
                l2_erc20_default_bridge: Default::default(),
                l1_weth_bridge: Default::default(),
                l2_weth_bridge: Default::default(),
                l2_legacy_shared_bridge: Default::default(),
            },
        };

        Ok(result)
    }

    pub async fn get_confirmed_tokens_impl(
        &self,
        from: u32,
        limit: u8,
    ) -> anyhow::Result<Vec<zksync_web3_decl::types::Token>> {
        let reader = self.read_inner()?;

        let fork_storage_read = reader
            .fork_storage
            .inner
            .read()
            .expect("failed reading fork storage");

        match fork_storage_read.fork.as_ref() {
            Some(fork) => Ok(fork
                .fork_source
                .get_confirmed_tokens(from, limit)
                .map_err(|e| {
                    anyhow::anyhow!("failed fetching bridge contracts from the fork: {:?}", e)
                })?),
            None => Ok(vec![zksync_web3_decl::types::Token {
                l1_address: Address::zero(),
                l2_address: L2_BASE_TOKEN_ADDRESS,
                name: "Ether".to_string(),
                symbol: "ETH".to_string(),
                decimals: 18,
            }]),
        }
    }

    pub async fn get_all_account_balances_impl(
        &self,
        address: Address,
    ) -> Result<HashMap<Address, U256>, Web3Error> {
        let inner = self.get_inner().clone();
        let tokens = self.get_confirmed_tokens_impl(0, 100).await?;

        let balances = {
            let writer = inner.write().map_err(|_e| {
                let error_message = "Failed to acquire lock. Please ensure the lock is not being held by another process or thread.".to_string();
                Web3Error::InternalError(anyhow::Error::msg(error_message))
            })?;
            let mut balances = HashMap::new();
            for token in tokens {
                let balance_key = storage_key_for_standard_token_balance(
                    AccountTreeId::new(token.l2_address),
                    &address,
                );
                let balance = match writer.fork_storage.read_value_internal(&balance_key) {
                    Ok(balance) => balance,
                    Err(error) => {
                        return Err(Web3Error::InternalError(anyhow::anyhow!(
                            "failed reading value: {:?}",
                            error
                        )));
                    }
                };
                if !balance.is_zero() {
                    balances.insert(token.l2_address, h256_to_u256(balance));
                }
            }
            balances
        };

        Ok(balances)
    }

    pub async fn get_block_details_impl(
        &self,
        block_number: L2BlockNumber,
    ) -> anyhow::Result<Option<BlockDetails>> {
        let base_system_contracts_hashes = self.system_contracts.base_system_contracts_hashes();
        let reader = self.read_inner()?;

        let maybe_block = reader
            .block_hashes
            .get(&(block_number.0 as u64))
            .and_then(|hash| reader.blocks.get(hash))
            .map(|block| BlockDetails {
                number: L2BlockNumber(block.number.as_u32()),
                l1_batch_number: L1BatchNumber(block.l1_batch_number.unwrap_or_default().as_u32()),
                base: BlockDetailsBase {
                    timestamp: block.timestamp.as_u64(),
                    l1_tx_count: 1,
                    l2_tx_count: block.transactions.len(),
                    root_hash: Some(block.hash),
                    status: BlockStatus::Verified,
                    commit_tx_hash: None,
                    commit_chain_id: None,
                    committed_at: None,
                    prove_tx_hash: None,
                    prove_chain_id: None,
                    proven_at: None,
                    execute_tx_hash: None,
                    execute_chain_id: None,
                    executed_at: None,
                    l1_gas_price: 0,
                    l2_fair_gas_price: reader.fee_input_provider.gas_price(),
                    fair_pubdata_price: Some(reader.fee_input_provider.fair_pubdata_price()),
                    base_system_contracts_hashes,
                },
                operator_address: Address::zero(),
                protocol_version: Some(ProtocolVersionId::latest()),
            })
            .or_else(|| {
                reader
                    .fork_storage
                    .inner
                    .read()
                    .expect("failed reading fork storage")
                    .fork
                    .as_ref()
                    .and_then(|fork| {
                        fork.fork_source
                            .get_block_details(block_number)
                            .ok()
                            .flatten()
                    })
            });

        Ok(maybe_block)
    }

    pub async fn get_transaction_details_impl(
        &self,
        hash: H256,
    ) -> anyhow::Result<Option<TransactionDetails>> {
        let reader = self.read_inner()?;

        let maybe_result = {
            reader
                .tx_results
                .get(&hash)
                .map(|TransactionResult { info, receipt, .. }| {
                    TransactionDetails {
                        is_l1_originated: false,
                        status: TransactionStatus::Included,
                        // if these are not set, fee is effectively 0
                        fee: receipt.effective_gas_price.unwrap_or_default()
                            * receipt.gas_used.unwrap_or_default(),
                        gas_per_pubdata: info.tx.common_data.fee.gas_per_pubdata_limit,
                        initiator_address: info.tx.initiator_account(),
                        received_at: utc_datetime_from_epoch_ms(info.tx.received_timestamp_ms),
                        eth_commit_tx_hash: None,
                        eth_prove_tx_hash: None,
                        eth_execute_tx_hash: None,
                    }
                })
                .or_else(|| {
                    reader
                        .fork_storage
                        .inner
                        .read()
                        .expect("failed reading fork storage")
                        .fork
                        .as_ref()
                        .and_then(|fork| {
                            fork.fork_source
                                .get_transaction_details(hash)
                                .ok()
                                .flatten()
                        })
                })
        };

        Ok(maybe_result)
    }

    pub async fn get_bytecode_by_hash_impl(&self, hash: H256) -> anyhow::Result<Option<Vec<u8>>> {
        let writer = self.write_inner()?;

        let maybe_bytecode = match writer.fork_storage.load_factory_dep_internal(hash) {
            Ok(maybe_bytecode) => maybe_bytecode,
            Err(error) => {
                return Err(anyhow::anyhow!("failed to load factory dep: {:?}", error));
            }
        };

        if maybe_bytecode.is_some() {
            return Ok(maybe_bytecode);
        }

        let maybe_fork_details = &writer
            .fork_storage
            .inner
            .read()
            .expect("failed reading fork storage")
            .fork;
        if let Some(fork_details) = maybe_fork_details {
            let maybe_bytecode = match fork_details.fork_source.get_bytecode_by_hash(hash) {
                Ok(maybe_bytecode) => maybe_bytecode,
                Err(error) => {
                    return Err(anyhow::anyhow!("failed to get bytecode: {:?}", error));
                }
            };

            Ok(maybe_bytecode)
        } else {
            Ok(None)
        }
    }

    pub async fn get_base_token_l1_address_impl(&self) -> anyhow::Result<Address> {
        Ok(H160::from_low_u64_be(1))
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use anvil_zksync_config::types::CacheConfig;
    use zksync_types::u256_to_h256;
    use zksync_types::{
        api::{self, Block, TransactionReceipt, TransactionVariant},
        transaction_request::CallRequest,
        Address, H160, H256,
    };

    use super::*;
    use crate::{
        fork::ForkDetails,
        node::InMemoryNode,
        testing,
        testing::{ForkBlockConfig, MockServer},
    };

    #[tokio::test]
    async fn test_estimate_fee() {
        let node = InMemoryNode::default();

        let mock_request = CallRequest {
            from: Some(
                "0xa61464658afeaf65cccaafd3a512b69a83b77618"
                    .parse()
                    .unwrap(),
            ),
            to: Some(
                "0x36615cf349d7f6344891b1e7ca7c72883f5dc049"
                    .parse()
                    .unwrap(),
            ),
            gas: Some(U256::from(0)),
            gas_price: Some(U256::from(0)),
            max_fee_per_gas: None,
            max_priority_fee_per_gas: None,
            value: Some(U256::from(0)),
            data: Some(vec![0, 0].into()),
            nonce: Some(U256::from(0)),
            transaction_type: None,
            access_list: None,
            eip712_meta: None,
            input: None,
        };

        let result = node.estimate_fee_impl(mock_request).await.unwrap();

        assert_eq!(result.gas_limit, U256::from(409123));
        assert_eq!(result.max_fee_per_gas, U256::from(45250000));
        assert_eq!(result.max_priority_fee_per_gas, U256::from(0));
        assert_eq!(result.gas_per_pubdata_limit, U256::from(3143));
    }

    #[tokio::test]
    async fn test_get_transaction_details_local() {
        // Arrange
        let node = InMemoryNode::default();
        let inner = node.get_inner();
        {
            let mut writer = inner.write().unwrap();
            writer.tx_results.insert(
                H256::repeat_byte(0x1),
                TransactionResult {
                    info: testing::default_tx_execution_info(),
                    receipt: TransactionReceipt {
                        logs: vec![],
                        gas_used: Some(U256::from(10_000)),
                        effective_gas_price: Some(U256::from(1_000_000_000)),
                        ..Default::default()
                    },
                    debug: testing::default_tx_debug_info(),
                },
            );
        }
        let result = node
            .get_transaction_details_impl(H256::repeat_byte(0x1))
            .await
            .expect("get transaction details")
            .expect("transaction details");

        // Assert
        assert!(matches!(result.status, TransactionStatus::Included));
        assert_eq!(result.fee, U256::from(10_000_000_000_000u64));
    }

    #[tokio::test]
    async fn test_get_transaction_details_fork() {
        let mock_server = MockServer::run_with_config(ForkBlockConfig {
            number: 10,
            transaction_count: 0,
            hash: H256::repeat_byte(0xab),
        });
        let input_tx_hash = H256::repeat_byte(0x02);
        mock_server.expect(
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 0,
                "method": "zks_getTransactionDetails",
                "params": [
                    format!("{:#x}", input_tx_hash),
                ],
            }),
            serde_json::json!({
                "jsonrpc": "2.0",
                "result": {
                    "isL1Originated": false,
                    "status": "included",
                    "fee": "0x74293f087500",
                    "gasPerPubdata": "0x4e20",
                    "initiatorAddress": "0x63ab285cd87a189f345fed7dd4e33780393e01f0",
                    "receivedAt": "2023-10-12T15:45:53.094Z",
                    "ethCommitTxHash": null,
                    "ethProveTxHash": null,
                    "ethExecuteTxHash": null
                },
                "id": 0
            }),
        );

        let node = InMemoryNode::default_fork(Some(
            ForkDetails::from_network(&mock_server.url(), None, &CacheConfig::None)
                .await
                .unwrap(),
        ));

        let result = node
            .get_transaction_details_impl(input_tx_hash)
            .await
            .expect("get transaction details")
            .expect("transaction details");

        assert!(matches!(result.status, TransactionStatus::Included));
        assert_eq!(result.fee, U256::from(127_720_500_000_000u64));
    }

    #[tokio::test]
    async fn test_get_block_details_local() {
        // Arrange
        let node = InMemoryNode::default();
        let inner = node.get_inner();
        {
            let mut writer = inner.write().unwrap();
            let block = Block::<TransactionVariant>::default();
            writer.blocks.insert(H256::repeat_byte(0x1), block);
            writer.block_hashes.insert(0, H256::repeat_byte(0x1));
        }
        let result = node
            .get_block_details_impl(L2BlockNumber(0))
            .await
            .expect("get block details")
            .expect("block details");

        // Assert
        assert!(matches!(result.number, L2BlockNumber(0)));
        assert_eq!(result.l1_batch_number, L1BatchNumber(0));
        assert_eq!(result.base.timestamp, 0);
    }

    #[tokio::test]
    async fn test_get_block_details_fork() {
        let mock_server = MockServer::run_with_config(ForkBlockConfig {
            number: 10,
            transaction_count: 0,
            hash: H256::repeat_byte(0xab),
        });
        let miniblock = L2BlockNumber::from(16474138);
        mock_server.expect(
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 0,
                "method": "zks_getBlockDetails",
                "params": [
                    miniblock.0,
                ],
            }),
            serde_json::json!({
                "jsonrpc": "2.0",
                "result": {
                  "number": 16474138,
                  "l1BatchNumber": 270435,
                  "timestamp": 1697405098,
                  "l1TxCount": 0,
                  "l2TxCount": 1,
                  "rootHash": "0xd9e60f9a684fd7fc16e87ae923341a6e4af24f286e76612efdfc2d55f3f4d064",
                  "status": "sealed",
                  "commitTxHash": null,
                  "committedAt": null,
                  "proveTxHash": null,
                  "provenAt": null,
                  "executeTxHash": null,
                  "executedAt": null,
                  "l1GasPrice": 6156252068u64,
                  "l2FairGasPrice": 50000000u64,
                  "fairPubdataPrice": 100u64,
                  "baseSystemContractsHashes": {
                    "bootloader": "0x0100089b8a2f2e6a20ba28f02c9e0ed0c13d702932364561a0ea61621f65f0a8",
                    "default_aa": "0x0100067d16a5485875b4249040bf421f53e869337fe118ec747cf40a4c777e5f"
                  },
                  "operatorAddress": "0xa9232040bf0e0aea2578a5b2243f2916dbfc0a69",
                  "protocolVersion": "Version15"
                },
                "id": 0
              }),
        );

        let node = InMemoryNode::default_fork(Some(
            ForkDetails::from_network(&mock_server.url(), None, &CacheConfig::None)
                .await
                .unwrap(),
        ));

        let result = node
            .get_block_details_impl(miniblock)
            .await
            .expect("get block details")
            .expect("block details");

        assert!(matches!(result.number, L2BlockNumber(16474138)));
        assert_eq!(result.l1_batch_number, L1BatchNumber(270435));
        assert_eq!(result.base.timestamp, 1697405098);
        assert_eq!(result.base.fair_pubdata_price, Some(100));
    }

    #[tokio::test]
    async fn test_get_bridge_contracts_uses_default_values_if_local() {
        // Arrange
        let node = InMemoryNode::default();
        let expected_bridge_addresses = BridgeAddresses {
            l1_shared_default_bridge: Default::default(),
            l2_shared_default_bridge: Default::default(),
            l1_erc20_default_bridge: Default::default(),
            l2_erc20_default_bridge: Default::default(),
            l1_weth_bridge: Default::default(),
            l2_weth_bridge: Default::default(),
            l2_legacy_shared_bridge: Default::default(),
        };

        let actual_bridge_addresses = node
            .get_bridge_contracts_impl()
            .await
            .expect("get bridge addresses");

        // Assert
        testing::assert_bridge_addresses_eq(&expected_bridge_addresses, &actual_bridge_addresses)
    }

    #[tokio::test]
    async fn test_get_bridge_contracts_uses_fork() {
        // Arrange
        let mock_server = MockServer::run_with_config(ForkBlockConfig {
            number: 10,
            transaction_count: 0,
            hash: H256::repeat_byte(0xab),
        });
        let input_bridge_addresses = BridgeAddresses {
            l1_shared_default_bridge: Some(H160::repeat_byte(0x1)),
            l2_shared_default_bridge: Some(H160::repeat_byte(0x2)),
            l1_erc20_default_bridge: Some(H160::repeat_byte(0x1)),
            l2_erc20_default_bridge: Some(H160::repeat_byte(0x2)),
            l1_weth_bridge: Some(H160::repeat_byte(0x3)),
            l2_weth_bridge: Some(H160::repeat_byte(0x4)),
            l2_legacy_shared_bridge: Some(H160::repeat_byte(0x6)),
        };
        mock_server.expect(
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 0,
                "method": "zks_getBridgeContracts",
            }),
            serde_json::json!({
                "jsonrpc": "2.0",
                "result": {
                    "l1Erc20SharedBridge": format!("{:#x}", input_bridge_addresses.l1_shared_default_bridge.unwrap()),
                    "l2Erc20SharedBridge": format!("{:#x}", input_bridge_addresses.l2_shared_default_bridge.unwrap()),
                    "l1Erc20DefaultBridge": format!("{:#x}", input_bridge_addresses.l1_erc20_default_bridge.unwrap()),
                    "l2Erc20DefaultBridge": format!("{:#x}", input_bridge_addresses.l2_erc20_default_bridge.unwrap()),
                    "l1WethBridge": format!("{:#x}", input_bridge_addresses.l1_weth_bridge.unwrap()),
                    "l2WethBridge": format!("{:#x}", input_bridge_addresses.l2_weth_bridge.unwrap())
                },
                "id": 0
            }),
        );

        let node = InMemoryNode::default_fork(Some(
            ForkDetails::from_network(&mock_server.url(), None, &CacheConfig::None)
                .await
                .unwrap(),
        ));

        let actual_bridge_addresses = node
            .get_bridge_contracts_impl()
            .await
            .expect("get bridge addresses");

        // Assert
        testing::assert_bridge_addresses_eq(&input_bridge_addresses, &actual_bridge_addresses)
    }

    #[tokio::test]
    async fn test_get_bytecode_by_hash_returns_local_value_if_available() {
        // Arrange
        let node = InMemoryNode::default();
        let input_hash = H256::repeat_byte(0x1);
        let input_bytecode = vec![0x1];
        node.get_inner()
            .write()
            .unwrap()
            .fork_storage
            .store_factory_dep(input_hash, input_bytecode.clone());

        let actual = node
            .get_bytecode_by_hash_impl(input_hash)
            .await
            .expect("failed fetching bytecode")
            .expect("no bytecode was found");

        // Assert
        assert_eq!(input_bytecode, actual);
    }

    #[tokio::test]
    async fn test_get_bytecode_by_hash_uses_fork_if_value_unavailable() {
        // Arrange
        let mock_server = MockServer::run_with_config(ForkBlockConfig {
            number: 10,
            transaction_count: 0,
            hash: H256::repeat_byte(0xab),
        });
        let input_hash = H256::repeat_byte(0x1);
        let input_bytecode = vec![0x1];
        mock_server.expect(
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 0,
                "method": "zks_getBytecodeByHash",
                "params": [
                    format!("{:#x}", input_hash)
                ],
            }),
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 0,
                "result": input_bytecode,
            }),
        );

        let node = InMemoryNode::default_fork(Some(
            ForkDetails::from_network(&mock_server.url(), None, &CacheConfig::None)
                .await
                .unwrap(),
        ));

        let actual = node
            .get_bytecode_by_hash_impl(input_hash)
            .await
            .expect("failed fetching bytecode")
            .expect("no bytecode was found");

        // Assert
        assert_eq!(input_bytecode, actual);
    }

    #[tokio::test]
    async fn test_get_raw_block_transactions_local() {
        // Arrange
        let node = InMemoryNode::default();
        let inner = node.get_inner();
        {
            let mut writer = inner.write().unwrap();
            let mut block = Block::<TransactionVariant>::default();
            let txn = api::Transaction::default();
            writer.tx_results.insert(
                txn.hash,
                TransactionResult {
                    info: testing::default_tx_execution_info(),
                    receipt: TransactionReceipt {
                        logs: vec![],
                        gas_used: Some(U256::from(10_000)),
                        effective_gas_price: Some(U256::from(1_000_000_000)),
                        ..Default::default()
                    },
                    debug: testing::default_tx_debug_info(),
                },
            );
            block.transactions.push(TransactionVariant::Full(txn));
            writer.blocks.insert(H256::repeat_byte(0x1), block);
            writer.block_hashes.insert(0, H256::repeat_byte(0x1));
        }

        let txns = node
            .get_raw_block_transactions_impl(L2BlockNumber(0))
            .await
            .expect("get transaction details");

        // Assert
        assert_eq!(txns.len(), 1);
    }

    #[tokio::test]
    async fn test_get_raw_block_transactions_fork() {
        let mock_server = MockServer::run_with_config(ForkBlockConfig {
            number: 10,
            transaction_count: 0,
            hash: H256::repeat_byte(0xab),
        });
        let miniblock = L2BlockNumber::from(16474138);
        mock_server.expect(
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 0,
                "method": "zks_getRawBlockTransactions",
                "params": [miniblock.0]
            }),
            serde_json::json!({
                "jsonrpc": "2.0",
                "result": [
                  {
                    "common_data": {
                      "L2": {
                        "nonce": 86,
                        "fee": {
                          "gas_limit": "0xcc626",
                          "max_fee_per_gas": "0x141dd760",
                          "max_priority_fee_per_gas": "0x0",
                          "gas_per_pubdata_limit": "0x4e20"
                        },
                        "initiatorAddress": "0x840bd73f903ba7dbb501be8326fe521dadcae1a5",
                        "signature": [
                          135,
                          163,
                          2,
                          78,
                          118,
                          14,
                          209
                        ],
                        "transactionType": "EIP1559Transaction",
                        "input": {
                          "hash": "0xc1f625f55d186ad0b439054adfe3317ae703c5f588f4fa1896215e8810a141e0",
                          "data": [
                            2,
                            249,
                            1,
                            110,
                            130
                          ]
                        },
                        "paymasterParams": {
                          "paymaster": "0x0000000000000000000000000000000000000000",
                          "paymasterInput": []
                        }
                      }
                    },
                    "execute": {
                      "contractAddress": "0xbe7d1fd1f6748bbdefc4fbacafbb11c6fc506d1d",
                      "calldata": "0x38ed173900000000000000000000000000000000000000000000000000000000002c34cc00000000000000000000000000000000000000000000000000000000002c9a2500000000000000000000000000000000000000000000000000000000000000a0000000000000000000000000840bd73f903ba7dbb501be8326fe521dadcae1a500000000000000000000000000000000000000000000000000000000652c5d1900000000000000000000000000000000000000000000000000000000000000020000000000000000000000008e86e46278518efc1c5ced245cba2c7e3ef115570000000000000000000000003355df6d4c9c3035724fd0e3914de96a5a83aaf4",
                      "value": "0x0",
                      "factoryDeps": null
                    },
                    "received_timestamp_ms": 1697405097873u64,
                    "raw_bytes": "0x02f9016e820144568084141dd760830cc62694be7d1fd1f6748bbdefc4fbacafbb11c6fc506d1d80b9010438ed173900000000000000000000000000000000000000000000000000000000002c34cc00000000000000000000000000000000000000000000000000000000002c9a2500000000000000000000000000000000000000000000000000000000000000a0000000000000000000000000840bd73f903ba7dbb501be8326fe521dadcae1a500000000000000000000000000000000000000000000000000000000652c5d1900000000000000000000000000000000000000000000000000000000000000020000000000000000000000008e86e46278518efc1c5ced245cba2c7e3ef115570000000000000000000000003355df6d4c9c3035724fd0e3914de96a5a83aaf4c080a087a3024e760ed14134ef541608bf308e083c899a89dba3c02bf3040f07c8b91b9fc3a7eeb6b3b8b36bb03ea4352415e7815dda4954f4898d255bd7660736285e"
                  }
                ],
                "id": 0
              }),
        );

        let node = InMemoryNode::default_fork(Some(
            ForkDetails::from_network(&mock_server.url(), None, &CacheConfig::None)
                .await
                .unwrap(),
        ));

        let txns = node
            .get_raw_block_transactions_impl(miniblock)
            .await
            .expect("get transaction details");
        assert_eq!(txns.len(), 1);
    }

    #[tokio::test]
    async fn test_get_all_account_balances_empty() {
        let node = InMemoryNode::default();
        let balances = node
            .get_all_account_balances_impl(Address::zero())
            .await
            .expect("get balances");
        assert!(balances.is_empty());
    }

    #[tokio::test]
    async fn test_get_confirmed_tokens_eth() {
        let node = InMemoryNode::default();
        let balances = node
            .get_confirmed_tokens_impl(0, 100)
            .await
            .expect("get balances");
        assert_eq!(balances.len(), 1);
        assert_eq!(&balances[0].name, "Ether");
    }

    #[tokio::test]
    async fn test_get_all_account_balances_forked() {
        let cbeth_address = Address::from_str("0x75af292c1c9a37b3ea2e6041168b4e48875b9ed5")
            .expect("failed to parse address");
        let mock_server = testing::MockServer::run();
        mock_server.expect(
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 0,
                "method": "eth_chainId",
            }),
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 0,
                "result": "0x104",
            }),
        );

        mock_server.expect(
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "zks_getBlockDetails",
                "params": [1]
            }),
            serde_json::json!({
                "jsonrpc": "2.0",
                "result": {
                    "baseSystemContractsHashes": {
                        "bootloader": "0x010008a5c30072f79f8e04f90b31f34e554279957e7e2bf85d3e9c7c1e0f834d",
                        "default_aa": "0x01000663d7941c097ba2631096508cf9ec7769ddd40e081fd81b0d04dc07ea0e"
                    },
                    "commitTxHash": null,
                    "committedAt": null,
                    "executeTxHash": null,
                    "executedAt": null,
                    "l1BatchNumber": 0,
                    "l1GasPrice": 0,
                    "l1TxCount": 1,
                    "l2FairGasPrice": 50000000,
                    "l2TxCount": 0,
                    "number": 0,
                    "operatorAddress": "0x0000000000000000000000000000000000000000",
                    "protocolVersion": "Version16",
                    "proveTxHash": null,
                    "provenAt": null,
                    "rootHash": "0xdaa77426c30c02a43d9fba4e841a6556c524d47030762eb14dc4af897e605d9b",
                    "status": "verified",
                    "timestamp": 1000
                },
                "id": 1
            }),
        );
        mock_server.expect(
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "eth_getBlockByHash",
                "params": ["0xdaa77426c30c02a43d9fba4e841a6556c524d47030762eb14dc4af897e605d9b", true]
            }),
            serde_json::json!({
                "jsonrpc": "2.0",
                "result": {
                    "baseFeePerGas": "0x0",
                    "difficulty": "0x0",
                    "extraData": "0x",
                    "gasLimit": "0xffffffff",
                    "gasUsed": "0x0",
                    "hash": "0xdaa77426c30c02a43d9fba4e841a6556c524d47030762eb14dc4af897e605d9b",
                    "l1BatchNumber": "0x0",
                    "l1BatchTimestamp": null,
                    "logsBloom": "0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
                    "miner": "0x0000000000000000000000000000000000000000",
                    "mixHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
                    "nonce": "0x0000000000000000",
                    "number": "0x0",
                    "parentHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
                    "receiptsRoot": "0x0000000000000000000000000000000000000000000000000000000000000000",
                    "sealFields": [],
                    "sha3Uncles": "0x0000000000000000000000000000000000000000000000000000000000000000",
                    "size": "0x0",
                    "stateRoot": "0x0000000000000000000000000000000000000000000000000000000000000000",
                    "timestamp": "0x3e8",
                    "totalDifficulty": "0x0",
                    "transactions": [],
                    "transactionsRoot": "0x0000000000000000000000000000000000000000000000000000000000000000",
                    "uncles": []
                },
                "id": 2
            }),
        );
        mock_server.expect(
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 0,
                "method": "zks_getConfirmedTokens",
                "params": [0, 100]
            }),
            serde_json::json!({
                "jsonrpc": "2.0",
                "result": [
                    {
                        "decimals": 18,
                        "l1Address": "0xbe9895146f7af43049ca1c1ae358b0541ea49704",
                        "l2Address": "0x75af292c1c9a37b3ea2e6041168b4e48875b9ed5",
                        "name": "Coinbase Wrapped Staked ETH",
                        "symbol": "cbETH"
                      }
                ],
                "id": 0
            }),
        );
        mock_server.expect(
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "zks_getFeeParams",
            }),
            serde_json::json!({
              "jsonrpc": "2.0",
              "result": {
                "V2": {
                  "config": {
                    "minimal_l2_gas_price": 25000000,
                    "compute_overhead_part": 0,
                    "pubdata_overhead_part": 1,
                    "batch_overhead_l1_gas": 800000,
                    "max_gas_per_batch": 200000000,
                    "max_pubdata_per_batch": 240000
                  },
                  "l1_gas_price": 46226388803u64,
                  "l1_pubdata_price": 100780475095u64
                }
              },
              "id": 3
            }),
        );

        let node = InMemoryNode::default_fork(Some(
            ForkDetails::from_network(&mock_server.url(), Some(1), &CacheConfig::None)
                .await
                .unwrap(),
        ));

        {
            let inner = node.get_inner();
            let writer = inner.write().unwrap();
            let mut fork = writer.fork_storage.inner.write().unwrap();
            fork.raw_storage.set_value(
                storage_key_for_standard_token_balance(
                    AccountTreeId::new(cbeth_address),
                    &Address::repeat_byte(0x1),
                ),
                u256_to_h256(U256::from(1337)),
            );
        }

        let balances = node
            .get_all_account_balances_impl(Address::repeat_byte(0x1))
            .await
            .expect("get balances");
        assert_eq!(balances.get(&cbeth_address).unwrap(), &U256::from(1337));
    }

    #[tokio::test]
    async fn test_get_base_token_l1_address() {
        let node = InMemoryNode::default();
        let token_address = node
            .get_base_token_l1_address_impl()
            .await
            .expect("get base token l1 address");
        assert_eq!(
            "0x0000000000000000000000000000000000000001",
            format!("{:?}", token_address)
        );
    }
}

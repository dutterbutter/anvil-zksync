use crate::deps::storage_view::StorageView;
use crate::node::{InMemoryNode, MAX_TX_SIZE};
use crate::utils::{create_debug_output, to_real_block_number};
use itertools::Itertools;
use once_cell::sync::OnceCell;
use std::sync::Arc;
use zksync_multivm::interface::{VmFactory, VmInterface};
use zksync_multivm::tracers::CallTracer;
use zksync_multivm::vm_latest::constants::ETH_CALL_GAS_LIMIT;
use zksync_multivm::vm_latest::{HistoryDisabled, ToTracerPointer, Vm};
use zksync_types::api::{
    BlockId, BlockNumber, CallTracerBlockResult, CallTracerResult, ResultDebugCall, TracerConfig,
    TransactionVariant,
};
use zksync_types::l2::L2Tx;
use zksync_types::transaction_request::CallRequest;
use zksync_types::{PackedEthSignature, Transaction, H256, U64};
use zksync_web3_decl::error::Web3Error;

impl InMemoryNode {
    pub async fn trace_block_by_number_impl(
        &self,
        block: BlockNumber,
        options: Option<TracerConfig>,
    ) -> anyhow::Result<CallTracerBlockResult> {
        let current_miniblock = self.read_inner()?.current_miniblock;
        let number = to_real_block_number(block, U64::from(current_miniblock)).as_u64();
        let block_hash = *self
            .read_inner()?
            .block_hashes
            .get(&number)
            .ok_or_else(|| anyhow::anyhow!("Block (id={block}) not found"))?;

        self.trace_block_by_hash_impl(block_hash, options).await
    }

    pub async fn trace_block_by_hash_impl(
        &self,
        hash: H256,
        options: Option<TracerConfig>,
    ) -> anyhow::Result<CallTracerBlockResult> {
        let only_top = options.is_some_and(|o| o.tracer_config.only_top_call);

        let tx_hashes = self
            .read_inner()?
            .blocks
            .get(&hash)
            .ok_or_else(|| anyhow::anyhow!("Block (hash={hash}) not found"))?
            .transactions
            .iter()
            .map(|tx| match tx {
                TransactionVariant::Full(tx) => tx.hash,
                TransactionVariant::Hash(hash) => *hash,
            })
            .collect_vec();

        let debug_calls = tx_hashes
            .into_iter()
            .map(|tx_hash| {
                Ok(self
                    .read_inner()?
                    .tx_results
                    .get(&tx_hash)
                    .ok_or_else(|| anyhow::anyhow!("Transaction (hash={tx_hash}) not found"))?
                    .debug_info(only_top))
            })
            .collect::<anyhow::Result<Vec<_>>>()?
            .into_iter()
            .map(|result| ResultDebugCall { result })
            .collect_vec();

        Ok(CallTracerBlockResult::CallTrace(debug_calls))
    }

    pub async fn trace_call_impl(
        &self,
        request: CallRequest,
        block: Option<BlockId>,
        options: Option<TracerConfig>,
    ) -> Result<CallTracerResult, Web3Error> {
        let only_top = options.is_some_and(|o| o.tracer_config.only_top_call);
        let inner = self.read_inner()?;
        let system_contracts = self.system_contracts.contracts_for_l2_call();
        if block.is_some() && !matches!(block, Some(BlockId::Number(BlockNumber::Latest))) {
            return Err(Web3Error::InternalError(anyhow::anyhow!(
                "tracing only supported at `latest` block"
            )));
        }

        let allow_no_target = system_contracts.evm_emulator.is_some();
        let mut l2_tx = L2Tx::from_request(request.into(), MAX_TX_SIZE, allow_no_target)
            .map_err(Web3Error::SerializationError)?;
        let execution_mode = zksync_multivm::interface::TxExecutionMode::EthCall;
        let storage = StorageView::new(&inner.fork_storage).into_rc_ptr();

        // init vm
        let (mut l1_batch_env, _block_context) =
            inner.create_l1_batch_env(&self.time, storage.clone());

        // update the enforced_base_fee within l1_batch_env to match the logic in zksync_core
        l1_batch_env.enforced_base_fee = Some(l2_tx.common_data.fee.max_fee_per_gas.as_u64());
        let system_env = inner.create_system_env(system_contracts.clone(), execution_mode);
        let mut vm: Vm<_, HistoryDisabled> = Vm::new(l1_batch_env, system_env, storage);

        // We must inject *some* signature (otherwise bootloader code fails to generate hash).
        if l2_tx.common_data.signature.is_empty() {
            l2_tx.common_data.signature = PackedEthSignature::default().serialize_packed().into();
        }

        // Match behavior of zksync_core:
        // Protection against infinite-loop eth_calls and alike:
        // limiting the amount of gas the call can use.
        l2_tx.common_data.fee.gas_limit = ETH_CALL_GAS_LIMIT.into();

        let tx: Transaction = l2_tx.clone().into();
        vm.push_transaction(tx);

        let call_tracer_result = Arc::new(OnceCell::default());
        let tracer = CallTracer::new(call_tracer_result.clone()).into_tracer_pointer();

        let tx_result = vm.inspect(
            &mut tracer.into(),
            zksync_multivm::interface::InspectExecutionMode::OneTx,
        );
        let call_traces = if only_top {
            vec![]
        } else {
            Arc::try_unwrap(call_tracer_result)
                .unwrap()
                .take()
                .unwrap_or_default()
        };

        let debug = create_debug_output(&l2_tx, &tx_result, call_traces)?;

        Ok(CallTracerResult::CallTrace(debug))
    }

    pub async fn trace_transaction_impl(
        &self,
        tx_hash: H256,
        options: Option<TracerConfig>,
    ) -> anyhow::Result<Option<CallTracerResult>> {
        let only_top = options.is_some_and(|o| o.tracer_config.only_top_call);
        let inner = self.read_inner()?;

        Ok(inner
            .tx_results
            .get(&tx_hash)
            .map(|tx| CallTracerResult::CallTrace(tx.debug_info(only_top))))
    }
}

#[cfg(test)]
mod tests {
    use anvil_zksync_config::constants::DEFAULT_ACCOUNT_BALANCE;
    use ethers::abi::{short_signature, AbiEncode, HumanReadableParser, ParamType, Token};
    use zksync_types::{
        api::{Block, CallTracerConfig, SupportedTracers, TransactionReceipt},
        transaction_request::CallRequestBuilder,
        utils::deployed_address_create,
        Address, K256PrivateKey, Nonce, H160, U256,
    };

    use super::*;
    use crate::{
        deps::system_contracts::bytecode_from_slice,
        node::{InMemoryNode, TransactionResult},
        testing::{self, LogBuilder},
    };

    fn deploy_test_contracts(node: &InMemoryNode) -> (Address, Address) {
        let private_key = K256PrivateKey::from_bytes(H256::repeat_byte(0xee)).unwrap();
        let from_account = private_key.address();
        node.set_rich_account(from_account, U256::from(DEFAULT_ACCOUNT_BALANCE));

        // first, deploy secondary contract
        let secondary_bytecode = bytecode_from_slice(
            "Secondary",
            include_bytes!("../deps/test-contracts/Secondary.json"),
        );
        let secondary_deployed_address = deployed_address_create(from_account, U256::zero());
        testing::deploy_contract(
            node,
            H256::repeat_byte(0x1),
            &private_key,
            secondary_bytecode,
            Some((U256::from(2),).encode()),
            Nonce(0),
        );

        // deploy primary contract using the secondary contract address as a constructor parameter
        let primary_bytecode = bytecode_from_slice(
            "Primary",
            include_bytes!("../deps/test-contracts/Primary.json"),
        );
        let primary_deployed_address = deployed_address_create(from_account, U256::one());
        testing::deploy_contract(
            node,
            H256::repeat_byte(0x1),
            &private_key,
            primary_bytecode,
            Some((secondary_deployed_address).encode()),
            Nonce(1),
        );
        (primary_deployed_address, secondary_deployed_address)
    }

    #[tokio::test]
    async fn test_trace_deployed_contract() {
        let node = InMemoryNode::default();

        let (primary_deployed_address, secondary_deployed_address) = deploy_test_contracts(&node);
        // trace a call to the primary contract
        let func = HumanReadableParser::parse_function("calculate(uint)").unwrap();
        let calldata = func.encode_input(&[Token::Uint(U256::from(42))]).unwrap();
        let request = CallRequestBuilder::default()
            .to(Some(primary_deployed_address))
            .data(calldata.clone().into())
            .gas(80_000_000.into())
            .build();
        let trace = node
            .trace_call_impl(request.clone(), None, None)
            .await
            .expect("trace call")
            .unwrap_default();

        // call should not revert
        assert!(trace.error.is_none());
        assert!(trace.revert_reason.is_none());

        // check that the call was successful
        let output =
            ethers::abi::decode(&[ParamType::Uint(256)], trace.output.0.as_slice()).unwrap();
        assert_eq!(output[0], Token::Uint(U256::from(84)));

        // find the call to primary contract in the trace
        let contract_call = trace
            .calls
            .first()
            .unwrap()
            .calls
            .last()
            .unwrap()
            .calls
            .first()
            .unwrap();

        assert_eq!(contract_call.to, primary_deployed_address);
        assert_eq!(contract_call.input, calldata.into());

        // check that it contains a call to secondary contract
        let subcall = contract_call.calls.first().unwrap();
        assert_eq!(subcall.to, secondary_deployed_address);
        assert_eq!(subcall.from, primary_deployed_address);
        assert_eq!(subcall.output, U256::from(84).encode().into());
    }

    #[tokio::test]
    async fn test_trace_only_top() {
        let node = InMemoryNode::default();

        let (primary_deployed_address, _) = deploy_test_contracts(&node);

        // trace a call to the primary contract
        let func = HumanReadableParser::parse_function("calculate(uint)").unwrap();
        let calldata = func.encode_input(&[Token::Uint(U256::from(42))]).unwrap();
        let request = CallRequestBuilder::default()
            .to(Some(primary_deployed_address))
            .data(calldata.into())
            .gas(80_000_000.into())
            .build();

        // if we trace with onlyTopCall=true, we should get only the top-level call
        let trace = node
            .trace_call_impl(
                request,
                None,
                Some(TracerConfig {
                    tracer: SupportedTracers::CallTracer,
                    tracer_config: CallTracerConfig {
                        only_top_call: true,
                    },
                }),
            )
            .await
            .expect("trace call")
            .unwrap_default();
        // call should not revert
        assert!(trace.error.is_none());
        assert!(trace.revert_reason.is_none());

        // call should not contain any subcalls
        assert!(trace.calls.is_empty());
    }

    #[tokio::test]
    async fn test_trace_reverts() {
        let node = InMemoryNode::default();

        let (primary_deployed_address, _) = deploy_test_contracts(&node);

        // trace a call to the primary contract
        let request = CallRequestBuilder::default()
            .to(Some(primary_deployed_address))
            .data(short_signature("shouldRevert()", &[]).into())
            .gas(80_000_000.into())
            .build();
        let trace = node
            .trace_call_impl(request, None, None)
            .await
            .expect("trace call")
            .unwrap_default();

        // call should revert
        assert!(trace.revert_reason.is_some());
        // find the call to primary contract in the trace
        let contract_call = trace
            .calls
            .first()
            .unwrap()
            .calls
            .last()
            .unwrap()
            .calls
            .first()
            .unwrap();

        // the contract subcall should have reverted
        assert!(contract_call.revert_reason.is_some());
    }

    #[tokio::test]
    async fn test_trace_transaction_impl() {
        let node = InMemoryNode::default();
        let inner = node.get_inner();
        {
            let mut writer = inner.write().unwrap();
            writer.tx_results.insert(
                H256::repeat_byte(0x1),
                TransactionResult {
                    info: testing::default_tx_execution_info(),
                    receipt: TransactionReceipt {
                        logs: vec![LogBuilder::new()
                            .set_address(H160::repeat_byte(0xa1))
                            .build()],
                        ..Default::default()
                    },
                    debug: testing::default_tx_debug_info(),
                },
            );
        }
        let result = node
            .trace_transaction_impl(H256::repeat_byte(0x1), None)
            .await
            .unwrap()
            .unwrap()
            .unwrap_default();
        assert_eq!(result.calls.len(), 1);
    }

    #[tokio::test]
    async fn test_trace_transaction_only_top() {
        let node = InMemoryNode::default();
        let inner = node.get_inner();
        {
            let mut writer = inner.write().unwrap();
            writer.tx_results.insert(
                H256::repeat_byte(0x1),
                TransactionResult {
                    info: testing::default_tx_execution_info(),
                    receipt: TransactionReceipt {
                        logs: vec![LogBuilder::new()
                            .set_address(H160::repeat_byte(0xa1))
                            .build()],
                        ..Default::default()
                    },
                    debug: testing::default_tx_debug_info(),
                },
            );
        }
        let result = node
            .trace_transaction_impl(
                H256::repeat_byte(0x1),
                Some(TracerConfig {
                    tracer: SupportedTracers::CallTracer,
                    tracer_config: CallTracerConfig {
                        only_top_call: true,
                    },
                }),
            )
            .await
            .unwrap()
            .unwrap()
            .unwrap_default();
        assert!(result.calls.is_empty());
    }

    #[tokio::test]
    async fn test_trace_transaction_not_found() {
        let node = InMemoryNode::default();
        let result = node
            .trace_transaction_impl(H256::repeat_byte(0x1), None)
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_trace_block_by_hash_empty() {
        let node = InMemoryNode::default();
        let inner = node.get_inner();
        {
            let mut writer = inner.write().unwrap();
            let block = Block::<TransactionVariant>::default();
            writer.blocks.insert(H256::repeat_byte(0x1), block);
        }
        let result = node
            .trace_block_by_hash_impl(H256::repeat_byte(0x1), None)
            .await
            .unwrap()
            .unwrap_default();
        assert_eq!(result.len(), 0);
    }

    #[tokio::test]
    async fn test_trace_block_by_hash_impl() {
        let node = InMemoryNode::default();
        let inner = node.get_inner();
        {
            let mut writer = inner.write().unwrap();
            let tx = zksync_types::api::Transaction::default();
            let tx_hash = tx.hash;
            let mut block = Block::<TransactionVariant>::default();
            block.transactions.push(TransactionVariant::Full(tx));
            writer.blocks.insert(H256::repeat_byte(0x1), block);
            writer.tx_results.insert(
                tx_hash,
                TransactionResult {
                    info: testing::default_tx_execution_info(),
                    receipt: TransactionReceipt::default(),
                    debug: testing::default_tx_debug_info(),
                },
            );
        }
        let result = node
            .trace_block_by_hash_impl(H256::repeat_byte(0x1), None)
            .await
            .unwrap()
            .unwrap_default();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].result.calls.len(), 1);
    }

    #[tokio::test]
    async fn test_trace_block_by_number_impl() {
        let node = InMemoryNode::default();
        let inner = node.get_inner();
        {
            let mut writer = inner.write().unwrap();
            let tx = zksync_types::api::Transaction::default();
            let tx_hash = tx.hash;
            let mut block = Block::<TransactionVariant>::default();
            block.transactions.push(TransactionVariant::Full(tx));
            writer.blocks.insert(H256::repeat_byte(0x1), block);
            writer.block_hashes.insert(0, H256::repeat_byte(0x1));
            writer.tx_results.insert(
                tx_hash,
                TransactionResult {
                    info: testing::default_tx_execution_info(),
                    receipt: TransactionReceipt::default(),
                    debug: testing::default_tx_debug_info(),
                },
            );
        }
        // check `latest` alias
        let result = node
            .trace_block_by_number_impl(BlockNumber::Latest, None)
            .await
            .unwrap()
            .unwrap_default();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].result.calls.len(), 1);

        // check block number
        let result = node
            .trace_block_by_number_impl(BlockNumber::Number(0.into()), None)
            .await
            .unwrap()
            .unwrap_default();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].result.calls.len(), 1);
    }
}
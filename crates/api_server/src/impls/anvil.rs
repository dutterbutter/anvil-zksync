use crate::error::RpcError;
use anvil_zksync_api_decl::AnvilNamespaceServer;
use anvil_zksync_core::node::InMemoryNode;
use anvil_zksync_types::api::{DetailedTransaction, ResetRequest};
use anvil_zksync_types::Numeric;
use jsonrpsee::core::RpcResult;
use zksync_types::api::Block;
use zksync_types::web3::Bytes;
use zksync_types::{Address, H256, U256, U64};

pub struct AnvilNamespace {
    node: InMemoryNode,
}

impl AnvilNamespace {
    pub fn new(node: InMemoryNode) -> Self {
        Self { node }
    }
}

impl AnvilNamespaceServer for AnvilNamespace {
    fn dump_state(&self, preserve_historical_states: Option<bool>) -> RpcResult<Bytes> {
        Ok(self
            .node
            .dump_state(preserve_historical_states.unwrap_or(false))
            .map_err(RpcError::from)?)
    }

    fn load_state(&self, bytes: Bytes) -> RpcResult<bool> {
        Ok(self.node.load_state(bytes).map_err(RpcError::from)?)
    }

    fn mine_detailed(&self) -> RpcResult<Block<DetailedTransaction>> {
        Ok(self.node.mine_detailed().map_err(RpcError::from)?)
    }

    fn set_rpc_url(&self, url: String) -> RpcResult<()> {
        Ok(self.node.set_rpc_url(url).map_err(RpcError::from)?)
    }

    fn set_next_block_base_fee_per_gas(&self, base_fee: U256) -> RpcResult<()> {
        Ok(self
            .node
            .set_next_block_base_fee_per_gas(base_fee)
            .map_err(RpcError::from)?)
    }

    fn drop_transaction(&self, hash: H256) -> RpcResult<Option<H256>> {
        Ok(self.node.drop_transaction(hash).map_err(RpcError::from)?)
    }

    fn drop_all_transactions(&self) -> RpcResult<()> {
        Ok(self.node.drop_all_transactions().map_err(RpcError::from)?)
    }

    fn remove_pool_transactions(&self, address: Address) -> RpcResult<()> {
        Ok(self
            .node
            .remove_pool_transactions(address)
            .map_err(RpcError::from)?)
    }

    fn get_auto_mine(&self) -> RpcResult<bool> {
        Ok(self.node.get_immediate_sealing().map_err(RpcError::from)?)
    }

    fn set_auto_mine(&self, enable: bool) -> RpcResult<()> {
        Ok(self
            .node
            .set_immediate_sealing(enable)
            .map_err(RpcError::from)?)
    }

    fn set_interval_mining(&self, seconds: u64) -> RpcResult<()> {
        Ok(self
            .node
            .set_interval_sealing(seconds)
            .map_err(RpcError::from)?)
    }

    fn set_block_timestamp_interval(&self, seconds: u64) -> RpcResult<()> {
        Ok(self
            .node
            .set_block_timestamp_interval(seconds)
            .map_err(RpcError::from)?)
    }

    fn remove_block_timestamp_interval(&self) -> RpcResult<bool> {
        Ok(self
            .node
            .remove_block_timestamp_interval()
            .map_err(RpcError::from)?)
    }

    fn set_min_gas_price(&self, _gas: U256) -> RpcResult<()> {
        tracing::info!(
            "Setting minimum gas price is unsupported as ZKsync is a post-EIP1559 chain"
        );
        Err(RpcError::Unsupported.into())
    }

    fn set_logging_enabled(&self, enable: bool) -> RpcResult<()> {
        Ok(self
            .node
            .set_logging_enabled(enable)
            .map_err(RpcError::from)?)
    }

    fn snapshot(&self) -> RpcResult<U64> {
        Ok(self.node.snapshot().map_err(RpcError::from)?)
    }

    fn revert(&self, id: U64) -> RpcResult<bool> {
        Ok(self.node.revert_snapshot(id).map_err(RpcError::from)?)
    }

    fn set_time(&self, timestamp: Numeric) -> RpcResult<i128> {
        Ok(self
            .node
            .set_time(timestamp.into())
            .map_err(RpcError::from)?)
    }

    fn increase_time(&self, seconds: Numeric) -> RpcResult<u64> {
        Ok(self
            .node
            .increase_time(seconds.into())
            .map_err(RpcError::from)?)
    }

    fn set_next_block_timestamp(&self, timestamp: Numeric) -> RpcResult<()> {
        Ok(self
            .node
            .set_next_block_timestamp(timestamp.into())
            .map_err(RpcError::from)?)
    }

    fn auto_impersonate_account(&self, enabled: bool) -> RpcResult<()> {
        self.node.auto_impersonate_account(enabled);
        Ok(())
    }

    fn set_balance(&self, address: Address, balance: U256) -> RpcResult<bool> {
        Ok(self
            .node
            .set_balance(address, balance)
            .map_err(RpcError::from)?)
    }

    fn set_nonce(&self, address: Address, nonce: U256) -> RpcResult<bool> {
        Ok(self
            .node
            .set_nonce(address, nonce)
            .map_err(RpcError::from)?)
    }

    fn anvil_mine(&self, num_blocks: Option<U64>, interval: Option<U64>) -> RpcResult<()> {
        Ok(self
            .node
            .mine_blocks(num_blocks, interval)
            .map_err(RpcError::from)?)
    }

    fn reset_network(&self, reset_spec: Option<ResetRequest>) -> RpcResult<bool> {
        Ok(self
            .node
            .reset_network(reset_spec)
            .map_err(RpcError::from)?)
    }

    fn impersonate_account(&self, address: Address) -> RpcResult<()> {
        Ok(self
            .node
            .impersonate_account(address)
            .map(|_| ())
            .map_err(RpcError::from)?)
    }

    fn stop_impersonating_account(&self, address: Address) -> RpcResult<()> {
        Ok(self
            .node
            .stop_impersonating_account(address)
            .map(|_| ())
            .map_err(RpcError::from)?)
    }

    fn set_code(&self, address: Address, code: String) -> RpcResult<()> {
        Ok(self.node.set_code(address, code).map_err(RpcError::from)?)
    }

    fn set_storage_at(&self, address: Address, slot: U256, value: U256) -> RpcResult<bool> {
        Ok(self
            .node
            .set_storage_at(address, slot, value)
            .map_err(RpcError::from)?)
    }

    fn set_chain_id(&self, id: u32) -> RpcResult<()> {
        Ok(self.node.set_chain_id(id).map_err(RpcError::from)?)
    }
}

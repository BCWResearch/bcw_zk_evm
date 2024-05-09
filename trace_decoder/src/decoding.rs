use std::{collections::HashMap, fmt::{self, Display, Formatter}, iter::once};

use ethereum_types::{Address, U256, U512};
use keccak_hash::H256;
use mpt_trie::{nibbles::Nibbles, partial_trie::HashedPartialTrie, trie_ops::TrieOpError};
use thiserror::Error;

use crate::{
    aliased_crate_types::{MptExtraBlockData, MptTrieInputs, MptTrieRoots}, compact::compact_processing_common::CompactParsingError, decoding_mpt::TxnMetaState, processed_block_trace::{NodesUsedByTxn, ProcessedBlockTrace, ProcessedSectionInfo, ProcessedSectionTxnInfo}, types::{HashedAccountAddr, HashedStorageAddr, OtherBlockData, StorageVal, TrieRootHash, TxnIdx, EMPTY_ACCOUNT_BYTES_RLPED, ZERO_STORAGE_SLOT_VAL_RLPED}, utils::{hash, optional_field, optional_field_hex}
};

pub type TraceParsingResult<T> = Result<T, Box<TraceParsingError>>;

pub(crate) trait ProcessedBlockTraceDecode {
    type Spec;
    type CurrBlockTries;
    type TrieInputs;
    type AccountRlp;
    type Ir;
    type TState: Clone + TrieState;

    fn get_trie_pre_image(spec: &Self::Spec) -> Self::TState;

    fn delete_node(h_addr: &Nibbles);

    fn create_trie_subsets(tries: &Self::CurrBlockTries) -> Self::TrieInputs;
}

pub(crate) trait TrieState {
    type AccountRlp;

    fn account_has_storage(&self, h_addr: &HashedAccountAddr) -> bool;
    fn write_account_data(&mut self, h_addr: HashedAccountAddr, data: Self::AccountRlp);
    fn delete_account(&mut self, h_addr: &HashedAccountAddr);

    fn set_storage_slot(&mut self, h_addr: HashedAccountAddr, h_slot: HashedAccountAddr, val: NodeInsertType);

    fn insert_receipt_node(&mut self, txn_idx: Nibbles, node_bytes: &[u8]);
    fn insert_txn_node(&mut self, txn_idx: Nibbles, node_bytes: &[u8]);
}

#[derive(Debug)]
pub(crate) enum NodeInsertType {
    Val(Vec<u8>),
    Hash(H256),
}

// TODO: Make this also work with SMT decoding...
/// Represents errors that can occur during the processing of a block trace.
///
/// This struct is intended to encapsulate various kinds of errors that might
/// arise when parsing, validating, or otherwise processing the trace data of
/// blockchain blocks. It could include issues like malformed trace data,
/// inconsistencies found during processing, or any other condition that
/// prevents successful completion of the trace processing task.
#[derive(Clone, Debug)]
pub struct TraceParsingError {
    block_num: Option<U256>,
    block_chain_id: Option<U256>,
    txn_idx: Option<usize>,
    addr: Option<Address>,
    h_addr: Option<H256>,
    slot: Option<U512>,
    slot_value: Option<U512>,
    reason: TraceParsingErrorReason, // The original error type
}

impl Display for TraceParsingError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let h_slot = self.slot.map(|slot| {
            let mut buf = [0u8; 64];
            slot.to_big_endian(&mut buf);
            hash(&buf)
        });
        write!(
            f,
            "Error processing trace: {}\n{}{}{}{}{}{}{}{}",
            self.reason,
            optional_field("Block num", self.block_num),
            optional_field("Block chain id", self.block_chain_id),
            optional_field("Txn idx", self.txn_idx),
            optional_field("Address", self.addr.as_ref()),
            optional_field("Hashed address", self.h_addr.as_ref()),
            optional_field_hex("Slot", self.slot),
            optional_field("Hashed Slot", h_slot),
            optional_field_hex("Slot value", self.slot_value),
        )
    }
}

impl std::error::Error for TraceParsingError {}

impl TraceParsingError {
    /// Function to create a new TraceParsingError with mandatory fields
    pub(crate) fn new(reason: TraceParsingErrorReason) -> Self {
        Self {
            block_num: None,
            block_chain_id: None,
            txn_idx: None,
            addr: None,
            h_addr: None,
            slot: None,
            slot_value: None,
            reason,
        }
    }

    /// Builder method to set block_num
    pub(crate) fn block_num(&mut self, block_num: U256) -> &mut Self {
        self.block_num = Some(block_num);
        self
    }

    /// Builder method to set block_chain_id
    pub(crate) fn block_chain_id(&mut self, block_chain_id: U256) -> &mut Self {
        self.block_chain_id = Some(block_chain_id);
        self
    }

    /// Builder method to set txn_idx
    pub fn txn_idx(&mut self, txn_idx: usize) -> &mut Self {
        self.txn_idx = Some(txn_idx);
        self
    }

    /// Builder method to set addr
    pub fn addr(&mut self, addr: Address) -> &mut Self {
        self.addr = Some(addr);
        self
    }

    /// Builder method to set h_addr
    pub fn h_addr(&mut self, h_addr: H256) -> &mut Self {
        self.h_addr = Some(h_addr);
        self
    }

    /// Builder method to set slot
    pub fn slot(&mut self, slot: U512) -> &mut Self {
        self.slot = Some(slot);
        self
    }

    /// Builder method to set slot_value
    pub fn slot_value(&mut self, slot_value: U512) -> &mut Self {
        self.slot_value = Some(slot_value);
        self
    }
}

/// An error reason for trie parsing.
#[derive(Clone, Debug, Error)]
pub enum TraceParsingErrorReason {
    /// Failure to decode an Ethereum [Account].
    #[error("Failed to decode RLP bytes ({0}) as an Ethereum account due to the error: {1}")]
    AccountDecode(String, String),

    /// Failure due to trying to access or delete a storage trie missing
    /// from the base trie.
    #[error("Missing account storage trie in base trie when constructing subset partial trie for txn (account: {0:x})")]
    MissingAccountStorageTrie(HashedAccountAddr),

    /// Failure due to trying to access a non-existent key in the trie.
    #[error("Tried accessing a non-existent key ({1:x}) in the {0} trie (root hash: {2:x})")]
    NonExistentTrieEntry(TrieType, Nibbles, TrieRootHash),

    /// Failure due to missing keys when creating a sub-partial trie.
    #[error("Missing key {0:x} when creating sub-partial tries (Trie type: {1})")]
    MissingKeysCreatingSubPartialTrie(Nibbles, TrieType),

    /// Failure due to trying to withdraw from a missing account
    #[error("No account present at {0:x} (hashed: {1:x}) to withdraw {2} Gwei from!")]
    MissingWithdrawalAccount(Address, HashedAccountAddr, U256),

    /// Failure due to a trie operation error.
    #[error("Trie operation error: {0}")]
    TrieOpError(TrieOpError),

    /// Failure due to a compact parsing error.
    #[error("Compact parsing error: {0}")]
    CompactParsingError(CompactParsingError),
}

impl From<TrieOpError> for TraceDecodingError {
    fn from(err: TrieOpError) -> Self {
        // Convert TrieOpError into TraceParsingError
        TraceDecodingError::new(TraceParsingErrorReason::TrieOpError(err))
    }
}

impl From<CompactParsingError> for TraceDecodingError {
    fn from(err: CompactParsingError) -> Self {
        // Convert CompactParsingError into TraceParsingError
        TraceDecodingError::new(TraceParsingErrorReason::CompactParsingError(err))
    }
}

impl From<TrieOpError> for TraceParsingError {
    fn from(err: TrieOpError) -> Self {
        // Convert TrieOpError into TraceParsingError
        TraceParsingError::new(TraceParsingErrorReason::TrieOpError(err))
    }
}

pub(crate) type TraceDecodingResult<T> = Result<T, Box<TraceDecodingError>>;

/// An enum to cover all Ethereum trie types (see https://ethereum.github.io/yellowpaper/paper.pdf for details).
#[derive(Clone, Copy, Debug)]
pub enum TrieType {
    /// State trie.
    State,
    /// Storage trie.
    Storage,
    /// Receipt trie.
    Receipt,
    /// Transaction trie.
    Txn,
}

impl Display for TrieType {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            TrieType::State => write!(f, "state"),
            TrieType::Storage => write!(f, "storage"),
            TrieType::Receipt => write!(f, "receipt"),
            TrieType::Txn => write!(f, "transaction"),
        }
    }
}

// TODO: Make this also work with SMT decoding...
/// Represents errors that can occur during the processing of a block trace.
///
/// This struct is intended to encapsulate various kinds of errors that might
/// arise when parsing, validating, or otherwise processing the trace data of
/// blockchain blocks. It could include issues like malformed trace data,
/// inconsistencies found during processing, or any other condition that
/// prevents successful completion of the trace processing task.
#[derive(Clone, Debug)]
pub struct TraceDecodingError {
    block_num: Option<U256>,
    block_chain_id: Option<U256>,
    txn_idx: Option<usize>,
    addr: Option<Address>,
    h_addr: Option<HashedAccountAddr>,
    slot: Option<U512>,
    slot_value: Option<U512>,
    reason: TraceParsingErrorReason, // The original error type
}

/// Additional information discovered during delta application.
#[derive(Debug, Default)]
struct TrieDeltaApplicationOutput {
    // During delta application, if a delete occurs, we may have to make sure additional nodes
    // that are not accessed by the txn remain unhashed.
    additional_state_trie_paths_to_not_hash: Vec<Nibbles>,
    additional_storage_trie_paths_to_not_hash: HashMap<H256, Vec<Nibbles>>,
}

impl Display for TraceDecodingError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let h_slot = self.slot.map(|slot| {
            let mut buf = [0u8; 64];
            slot.to_big_endian(&mut buf);
            hash(&buf)
        });
        write!(
            f,
            "Error processing trace: {}\n{}{}{}{}{}{}{}{}",
            self.reason,
            optional_field("Block num", self.block_num),
            optional_field("Block chain id", self.block_chain_id),
            optional_field("Txn idx", self.txn_idx),
            optional_field("Address", self.addr.as_ref()),
            optional_field("Hashed address", self.h_addr.as_ref()),
            optional_field_hex("Slot", self.slot),
            optional_field("Hashed Slot", h_slot),
            optional_field_hex("Slot value", self.slot_value),
        )
    }
}

impl std::error::Error for TraceDecodingError {}

// TODO: Remove public accessors once all PRs for SMTs stuff is merged in...
impl TraceDecodingError {
    /// Function to create a new TraceParsingError with mandatory fields
    pub(crate) fn new(reason: TraceParsingErrorReason) -> Self {
        Self {
            block_num: None,
            block_chain_id: None,
            txn_idx: None,
            addr: None,
            h_addr: None,
            slot: None,
            slot_value: None,
            reason,
        }
    }

    /// Builder method to set block_num
    pub(crate) fn block_num(&mut self, block_num: U256) -> &mut Self {
        self.block_num = Some(block_num);
        self
    }

    /// Builder method to set block_chain_id
    pub(crate) fn block_chain_id(&mut self, block_chain_id: U256) -> &mut Self {
        self.block_chain_id = Some(block_chain_id);
        self
    }

    /// Builder method to set txn_idx
    pub(crate) fn txn_idx(&mut self, txn_idx: usize) -> &mut Self {
        self.txn_idx = Some(txn_idx);
        self
    }

    /// Builder method to set addr
    pub(crate) fn addr(&mut self, addr: Address) -> &mut Self {
        self.addr = Some(addr);
        self
    }

    /// Builder method to set h_addr
    pub(crate) fn h_addr(&mut self, h_addr: H256) -> &mut Self {
        self.h_addr = Some(h_addr);
        self
    }

    /// Builder method to set slot
    pub(crate) fn slot(&mut self, slot: U512) -> &mut Self {
        self.slot = Some(slot);
        self
    }

    /// Builder method to set slot_value
    pub(crate) fn slot_value(&mut self, slot_value: U512) -> &mut Self {
        self.slot_value = Some(slot_value);
        self
    }
}

impl<T, D> ProcessedBlockTrace<T, D>
where
    D: ProcessedBlockTraceDecode<Spec = T>
{

    pub(crate) fn into_proof_gen_ir(
        self,
        other_data: OtherBlockData,
    ) -> TraceParsingResult<Vec<D::Ir>> {
        match self.sect_info {
            ProcessedSectionInfo::Continuations(_) => {
                todo!("MPT continuations are not implemented yet!")
            }
            ProcessedSectionInfo::Txns(txns) => {
                Self::process_txns(txns, D::get_trie_pre_image(&self.spec), self.withdrawals, &other_data)
            }
        }
    }

    fn process_txns(
        txns: Vec<ProcessedSectionTxnInfo>,
        tries: D::TState,
        withdrawals: Vec<(Address, U256)>,
        other_data: &OtherBlockData,
    ) -> TraceParsingResult<Vec<D::Ir>> {
        let mut curr_block_tries = tries;

        // This is just a copy of `curr_block_tries`.
        // TODO: Check if we can remove these clones before PR merge...
        let initial_tries_for_dummies = curr_block_tries.clone();

        let mut extra_data = MptExtraBlockData {
            checkpoint_state_trie_root: other_data.checkpoint_state_trie_root,
            txn_number_before: U256::zero(),
            txn_number_after: U256::zero(),
            gas_used_before: U256::zero(),
            gas_used_after: U256::zero(),
        };

        // A copy of the initial extra_data possibly needed during padding.
        let extra_data_for_dummies = extra_data.clone();

        let mut ir = txns
            .into_iter()
            .enumerate()
            .map(|(txn_idx, sect_info)| {
                Self::process_txn_info(
                    txn_idx,
                    sect_info,
                    &mut curr_block_tries,
                    &mut extra_data,
                    other_data,
                )
                .map_err(|mut e| {
                    e.txn_idx(txn_idx);
                    e
                })
            })
            .collect::<TraceDecodingResult<Vec<_>>>()
            .map_err(|mut e| {
                e.block_num(other_data.b_data.b_meta.block_number);
                e.block_chain_id(other_data.b_data.b_meta.block_chain_id);
                e
            })?;

        Self::pad_gen_inputs_with_dummy_inputs_if_needed(
            &mut ir,
            other_data,
            &extra_data,
            &extra_data_for_dummies,
            &initial_tries_for_dummies,
            &curr_block_tries,
        );

        if !withdrawals.is_empty() {
            Self::add_withdrawals_to_txns(&mut ir, &mut curr_block_tries, withdrawals.clone())?;
        }

        Ok(ir)
    }

    fn update_txn_and_receipt_tries(
        trie_state: &mut D::TState,
        meta: &TxnMetaState,
        txn_idx: TxnIdx,
    ) {
        let txn_k = Nibbles::from_bytes_be(&rlp::encode(&txn_idx)).unwrap();

        trie_state.insert_txn_node(txn_k, &meta.txn_bytes());
        trie_state.insert_receipt_node(txn_k, meta.receipt_node_bytes.as_ref());
    }

    /// If the account does not have a storage trie or does but is not
    /// accessed by any txns, then we still need to manually create an entry for
    /// them.
    fn init_any_needed_empty_storage_tries<'a>(
        trie_state: &mut D::TState,
        accounts_with_storage: impl Iterator<Item = &'a HashedStorageAddr>,
        state_accounts_with_no_accesses_but_storage_tries: &'a HashMap<
            HashedAccountAddr,
            TrieRootHash,
        >,
    ) {
        for h_addr in accounts_with_storage {

            if !trie_state.account_has_storage(h_addr) {
                trie_state.set_storage_slot(h_addr, h_slot, val)

                let trie = state_accounts_with_no_accesses_but_storage_tries
                    .get(h_addr)
                    .map(|s_root| HashedPartialTrie::new(Node::Hash(*s_root)))
                    .unwrap_or_default();

                storage_tries.insert(*h_addr, trie);
            };
        }
    }

    fn apply_deltas_to_trie_state(
        trie_state: &mut D::TState,
        deltas: &NodesUsedByTxn,
        meta: &TxnMetaState,
    ) -> TraceDecodingResult<TrieDeltaApplicationOutput> {
        let mut out = TrieDeltaApplicationOutput::default();

        for (hashed_acc_addr, storage_writes) in deltas.storage_writes.iter() {
            let mut storage_trie =
                trie_state.storage.get_mut(hashed_acc_addr).ok_or_else(|| {
                    let hashed_acc_addr = *hashed_acc_addr;
                    let mut e = TraceParsingError::new(
                        TraceParsingErrorReason::MissingAccountStorageTrie(hashed_acc_addr),
                    );
                    e.h_addr(hashed_acc_addr);
                    e
                })?;

            for (slot, val) in storage_writes
                .iter()
                .map(|(k, v)| (Nibbles::from_h256_be(hash(&k.bytes_be())), v))
            {
                // If we are writing a zero, then we actually need to perform a delete.
                match val == &ZERO_STORAGE_SLOT_VAL_RLPED {
                    false => storage_trie.insert(slot, val.clone()).map_err(|err| {
                        let mut e =
                            TraceParsingError::new(TraceParsingErrorReason::TrieOpError(err));
                        e.slot(U512::from_big_endian(slot.bytes_be().as_slice()));
                        e.slot_value(U512::from_big_endian(val.as_slice()));
                        e
                    })?,
                    true => {
                        if let Some(remaining_slot_key) =
                            Self::delete_node_and_report_remaining_key_if_branch_collapsed(
                                storage_trie,
                                &slot,
                            )
                        {
                            out.additional_storage_trie_paths_to_not_hash
                                .entry(*hashed_acc_addr)
                                .or_default()
                                .push(remaining_slot_key);
                        }
                    }
                };
            }
        }

        for (hashed_acc_addr, s_trie_writes) in deltas.state_writes.iter() {
            let val_k = Nibbles::from_h256_be(*hashed_acc_addr);

            // If the account was created, then it will not exist in the trie.
            let val_bytes = trie_state
                .state
                .get(val_k)
                .unwrap_or(&EMPTY_ACCOUNT_BYTES_RLPED);

            let mut account = account_from_rlped_bytes(val_bytes)?;

            s_trie_writes.apply_writes_to_state_node(
                &mut account,
                hashed_acc_addr,
                &trie_state.storage,
            )?;

            let updated_account_bytes = rlp::encode(&account);
            trie_state
                .state
                .insert(val_k, updated_account_bytes.to_vec());
        }

        // Remove any accounts that self-destructed.
        for hashed_addr in deltas.self_destructed_accounts.iter() {
            let k = Nibbles::from_h256_be(*hashed_addr);

            trie_state.storage.remove(hashed_addr).ok_or_else(|| {
                let hashed_addr = *hashed_addr;
                let mut e = TraceParsingError::new(
                    TraceParsingErrorReason::MissingAccountStorageTrie(hashed_addr),
                );
                e.h_addr(hashed_addr);
                e
            })?;

            // TODO: Once the mechanism for resolving code hashes settles, we probably want
            // to also delete the code hash mapping here as well...

            if let Some(remaining_account_key) =
                Self::delete_node_and_report_remaining_key_if_branch_collapsed(
                    &mut trie_state.state,
                    &k,
                )
            {
                out.additional_state_trie_paths_to_not_hash
                    .push(remaining_account_key);
            }
        }

        Ok(out)
    }

    /// Pads a generated IR vec with additional "dummy" entries if needed.
    /// We need to ensure that generated IR always has at least `2` elements,
    /// and if there are only `0` or `1` elements, then we need to pad so
    /// that we have two entries in total. These dummy entries serve only to
    /// allow the proof generation process to finish. Specifically, we need
    /// at least two entries to generate an agg proof, and we need an agg
    /// proof to generate a block proof. These entries do not mutate state.
    fn pad_gen_inputs_with_dummy_inputs_if_needed(
        gen_inputs: &mut Vec<GenerationInputs>,
        other_data: &OtherBlockData,
        final_extra_data: &MptExtraBlockData,
        initial_extra_data: &MptExtraBlockData,
        initial_tries: &PartialTrieState,
        final_tries: &PartialTrieState,
    ) {
        match gen_inputs.len() {
            0 => {
                debug_assert!(initial_tries.state == final_tries.state);
                debug_assert!(initial_extra_data == final_extra_data);
                // We need to pad with two dummy entries.
                gen_inputs.extend(create_dummy_txn_pair_for_empty_block(
                    other_data,
                    final_extra_data,
                    initial_tries,
                ));
            }
            1 => {
                // We just need one dummy entry.
                // The dummy proof will be prepended to the actual txn.
                let dummy_txn =
                    create_dummy_gen_input(other_data, initial_extra_data, initial_tries);
                gen_inputs.insert(0, dummy_txn)
            }
            _ => (),
        }
    }

    /// The withdrawals are always in the final ir payload.
    fn add_withdrawals_to_txns(
        txn_ir: &mut [GenerationInputs],
        final_trie_state: &mut PartialTrieState,
        withdrawals: Vec<(Address, U256)>,
    ) -> MptTraceParsingResult<()> {
        let withdrawals_with_hashed_addrs_iter = || {
            withdrawals
                .iter()
                .map(|(addr, v)| (*addr, hash(addr.as_bytes()), *v))
        };

        let last_inputs = txn_ir
            .last_mut()
            .expect("We cannot have an empty list of payloads.");

        if last_inputs.signed_txn.is_none() {
            // This is a dummy payload, hence it does not contain yet
            // state accesses to the withdrawal addresses.
            let withdrawal_addrs =
                withdrawals_with_hashed_addrs_iter().map(|(_, h_addr, _)| h_addr);
            last_inputs.tries.state_trie = create_minimal_state_partial_trie(
                &last_inputs.tries.state_trie,
                withdrawal_addrs,
                iter::empty(),
            )?;
        }

        Self::update_trie_state_from_withdrawals(
            withdrawals_with_hashed_addrs_iter(),
            &mut final_trie_state.state,
        )?;

        last_inputs.withdrawals = withdrawals;
        last_inputs.trie_roots_after.state_root = final_trie_state.state.hash();

        Ok(())
    }

    /// Withdrawals update balances in the account trie, so we need to update
    /// our local trie state.
    fn update_trie_state_from_withdrawals<'a>(
        withdrawals: impl IntoIterator<Item = (Address, HashedAccountAddr, U256)> + 'a,
        state: &mut HashedPartialTrie,
    ) -> MptTraceParsingResult<()> {
        for (addr, h_addr, amt) in withdrawals {
            let h_addr_nibs = Nibbles::from_h256_be(h_addr);

            let acc_bytes = state.get(h_addr_nibs).ok_or_else(|| {
                let mut e = TraceParsingError::new(
                    TraceParsingErrorReason::MissingWithdrawalAccount(addr, h_addr, amt),
                );
                e.addr(addr);
                e.h_addr(h_addr);
                e
            })?;
            let mut acc_data = account_from_rlped_bytes(acc_bytes)?;

            acc_data.balance += amt;

            state.insert(h_addr_nibs, rlp::encode(&acc_data).to_vec());
        }

        Ok(())
    }

    /// Processes a single transaction in the trace.
    fn process_txn_info(
        txn_idx: usize,
        txn_info: ProcessedSectionTxnInfo,
        curr_block_tries: &mut PartialTrieState,
        extra_data: &mut MptExtraBlockData,
        other_data: &OtherBlockData,
    ) -> MptTraceParsingResult<GenerationInputs> {
        trace!("Generating proof IR for txn {}...", txn_idx);

        Self::init_any_needed_empty_storage_tries(
            &mut curr_block_tries.storage,
            txn_info
                .nodes_used_by_txn
                .storage_accesses
                .iter()
                .map(|(k, _)| k),
            &txn_info
                .nodes_used_by_txn
                .state_accounts_with_no_accesses_but_storage_tries,
        );
        // For each non-dummy txn, we increment `txn_number_after` by 1, and
        // update `gas_used_after` accordingly.
        extra_data.txn_number_after += U256::one();
        extra_data.gas_used_after += txn_info.meta.gas_used.into();

        // Because we need to run delta application before creating the minimal
        // sub-tries (we need to detect if deletes collapsed any branches), we need to
        // do this clone every iteration.
        let tries_at_start_of_txn = curr_block_tries.clone();

        Self::update_txn_and_receipt_tries(curr_block_tries, &txn_info.meta, txn_idx);

        let delta_out = Self::apply_deltas_to_trie_state(
            curr_block_tries,
            &txn_info.nodes_used_by_txn,
            &txn_info.meta,
        )?;

        let tries = Self::create_minimal_partial_tries_needed_by_txn(
            &tries_at_start_of_txn,
            &txn_info.nodes_used_by_txn,
            txn_idx,
            delta_out,
            &other_data.b_data.b_meta.block_beneficiary,
        )?;

        let trie_roots_after = calculate_trie_input_hashes(curr_block_tries);
        let gen_inputs = GenerationInputs {
            txn_number_before: extra_data.txn_number_before,
            gas_used_before: extra_data.gas_used_before,
            gas_used_after: extra_data.gas_used_after,
            signed_txn: txn_info.meta.txn_bytes,
            withdrawals: Vec::default(), /* Only ever set in a dummy txn at the end of
                                          * the block (see `[add_withdrawals_to_txns]`
                                          * for more info). */
            tries,
            trie_roots_after,
            checkpoint_state_trie_root: extra_data.checkpoint_state_trie_root,
            contract_code: txn_info.contract_code_accessed,
            block_metadata: other_data.b_data.b_meta.clone(),
            block_hashes: other_data.b_data.b_hashes.clone(),
        };

        // After processing a transaction, we update the remaining accumulators
        // for the next transaction.
        extra_data.txn_number_before += U256::one();
        extra_data.gas_used_before = extra_data.gas_used_after;

        Ok(gen_inputs)
    }
}

impl StateTrieWrites {
    fn apply_writes_to_state_node(
        &self,
        state_node: &mut MptAccountRlp,
        h_addr: &HashedAccountAddr,
        acc_storage_tries: &HashMap<HashedAccountAddr, HashedPartialTrie>,
    ) -> MptTraceParsingResult<()> {
        let storage_root_hash_change = match self.storage_trie_change {
            false => None,
            true => {
                let storage_trie = acc_storage_tries.get(h_addr).ok_or_else(|| {
                    let h_addr = *h_addr;
                    let mut e = TraceParsingError::new(
                        TraceParsingErrorReason::MissingAccountStorageTrie(h_addr),
                    );
                    e.h_addr(h_addr);
                    e
                })?;

                Some(storage_trie.hash())
            }
        };

        update_val_if_some(&mut state_node.balance, self.balance);
        update_val_if_some(&mut state_node.nonce, self.nonce);
        update_val_if_some(&mut state_node.storage_root, storage_root_hash_change);
        update_val_if_some(&mut state_node.code_hash, self.code_hash);

        Ok(())
    }
}

fn calculate_trie_input_hashes(t_inputs: &PartialTrieState) -> MptTrieRoots {
    MptTrieRoots {
        state_root: t_inputs.state.hash(),
        transactions_root: t_inputs.txn.hash(),
        receipts_root: t_inputs.receipt.hash(),
    }
}

// We really want to get a trie with just a hash node here, and this is an easy
// way to do it.
fn create_fully_hashed_out_sub_partial_trie(trie: &HashedPartialTrie) -> HashedPartialTrie {
    // Impossible to actually fail with an empty iter.
    create_trie_subset(trie, empty::<Nibbles>()).unwrap()
}

fn create_dummy_txn_pair_for_empty_block(
    other_data: &OtherBlockData,
    extra_data: &MptExtraBlockData,
    final_tries: &PartialTrieState,
) -> [GenerationInputs; 2] {
    [
        create_dummy_gen_input(other_data, extra_data, final_tries),
        create_dummy_gen_input(other_data, extra_data, final_tries),
    ]
}

fn create_dummy_gen_input(
    other_data: &OtherBlockData,
    extra_data: &MptExtraBlockData,
    final_tries: &PartialTrieState,
) -> GenerationInputs {
    let sub_tries = create_dummy_proof_trie_inputs(
        final_tries,
        create_fully_hashed_out_sub_partial_trie(&final_tries.state),
    );
    create_dummy_gen_input_common(other_data, extra_data, sub_tries)
}

fn create_dummy_gen_input_with_state_addrs_accessed(
    other_data: &OtherBlockData,
    extra_data: &MptExtraBlockData,
    final_tries: &PartialTrieState,
    account_addrs_accessed: impl Iterator<Item = HashedAccountAddr>,
) -> MptTraceParsingResult<GenerationInputs> {
    let sub_tries = create_dummy_proof_trie_inputs(
        final_tries,
        create_minimal_state_partial_trie(
            &final_tries.state,
            account_addrs_accessed,
            iter::empty(),
        )?,
    );
    Ok(create_dummy_gen_input_common(
        other_data, extra_data, sub_tries,
    ))
}

fn create_dummy_gen_input_common(
    other_data: &OtherBlockData,
    extra_data: &MptExtraBlockData,
    sub_tries: MptTrieInputs,
) -> GenerationInputs {
    let trie_roots_after = MptTrieRoots {
        state_root: sub_tries.state_trie.hash(),
        transactions_root: sub_tries.transactions_trie.hash(),
        receipts_root: sub_tries.receipts_trie.hash(),
    };

    // Sanity checks
    assert_eq!(
        extra_data.txn_number_before, extra_data.txn_number_after,
        "Txn numbers before/after differ in a dummy payload with no txn!"
    );
    assert_eq!(
        extra_data.gas_used_before, extra_data.gas_used_after,
        "Gas used before/after differ in a dummy payload with no txn!"
    );

    GenerationInputs {
        signed_txn: None,
        tries: sub_tries,
        trie_roots_after,
        checkpoint_state_trie_root: extra_data.checkpoint_state_trie_root,
        block_metadata: other_data.b_data.b_meta.clone(),
        block_hashes: other_data.b_data.b_hashes.clone(),
        txn_number_before: extra_data.txn_number_before,
        gas_used_before: extra_data.gas_used_before,
        gas_used_after: extra_data.gas_used_after,
        contract_code: HashMap::default(),
        withdrawals: vec![], // this is set after creating dummy payloads
    }
}

fn create_dummy_proof_trie_inputs(
    final_tries_at_end_of_block: &PartialTrieState,
    state_trie: HashedPartialTrie,
) -> MptTrieInputs {
    let partial_sub_storage_tries: Vec<_> = final_tries_at_end_of_block
        .storage
        .iter()
        .map(|(hashed_acc_addr, s_trie)| {
            (
                *hashed_acc_addr,
                create_fully_hashed_out_sub_partial_trie(s_trie),
            )
        })
        .collect();

    MptTrieInputs {
        state_trie,
        transactions_trie: create_fully_hashed_out_sub_partial_trie(
            &final_tries_at_end_of_block.txn,
        ),
        receipts_trie: create_fully_hashed_out_sub_partial_trie(
            &final_tries_at_end_of_block.receipt,
        ),
        storage_tries: partial_sub_storage_tries,
    }
}

#[derive(Debug, Default)]
pub(crate) struct TxnMetaState {
    pub(crate) txn_bytes: Option<Vec<u8>>,
    pub(crate) receipt_node_bytes: Vec<u8>,
    pub(crate) gas_used: u64,
}

impl TxnMetaState {
    fn txn_bytes(&self) -> Vec<u8> {
        match self.txn_bytes.as_ref() {
            Some(v) => v.clone(),
            None => Vec::default(),
        }
    }
}

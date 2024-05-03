use plonky2::field::extension::Extendable;
use plonky2::field::polynomial::PolynomialValues;
use plonky2::hash::hash_types::RichField;
use plonky2::timed;
use plonky2::util::timing::TimingTree;
use starky::config::StarkConfig;
use starky::util::trace_rows_to_poly_values;

use crate::all_stark::{AllStark, NUM_TABLES};
use crate::arithmetic::{BinaryOperator, Operation};
use crate::byte_packing::byte_packing_stark::BytePackingOp;
use crate::cpu::columns::CpuColumnsView;
use crate::keccak_sponge::keccak_sponge_stark::KeccakSpongeOp;
use crate::witness::memory::MemoryOp;
use crate::{arithmetic, keccak, keccak_sponge, logic};

#[derive(Clone, Copy, Debug)]
pub(crate) struct TraceCheckpoint {
    pub(self) arithmetic_len: usize,
    pub(self) byte_packing_len: usize,
    pub(self) cpu_len: usize,
    pub(self) keccak_len: usize,
    pub(self) keccak_sponge_len: usize,
    pub(self) logic_len: usize,
    pub(self) memory_len: usize,
}

#[derive(Debug)]
pub(crate) struct Traces<T: Copy> {
    pub(crate) arithmetic_ops: Vec<arithmetic::Operation>,
    pub(crate) byte_packing_ops: Vec<BytePackingOp>,
    pub(crate) cpu: Vec<CpuColumnsView<T>>,
    pub(crate) logic_ops: Vec<logic::Operation>,
    pub(crate) memory_ops: Vec<MemoryOp>,
    pub(crate) keccak_inputs: Vec<([u64; keccak::keccak_stark::NUM_INPUTS], usize)>,
    pub(crate) keccak_sponge_ops: Vec<KeccakSpongeOp>,
}

impl<T: Copy> Traces<T> {
    pub(crate) fn new() -> Self {
        Traces {
            arithmetic_ops: vec![],
            byte_packing_ops: vec![],
            cpu: vec![],
            logic_ops: vec![],
            memory_ops: vec![],
            keccak_inputs: vec![],
            keccak_sponge_ops: vec![],
        }
    }

    /// Returns the actual trace lengths for each STARK module.
    //  Uses a `TraceCheckPoint` as return object for convenience.
    pub(crate) fn get_lengths(&self) -> TraceCheckpoint {
        TraceCheckpoint {
            arithmetic_len: self
                .arithmetic_ops
                .iter()
                .map(|op| match op {
                    Operation::TernaryOperation { .. } => 2,
                    Operation::BinaryOperation { operator, .. } => match operator {
                        BinaryOperator::Div | BinaryOperator::Mod => 2,
                        _ => 1,
                    },
                    Operation::RangeCheckOperation { .. } => 1,
                })
                .sum(),
            byte_packing_len: self
                .byte_packing_ops
                .iter()
                .map(|op| usize::from(!op.bytes.is_empty()))
                .sum(),
            cpu_len: self.cpu.len(),
            keccak_len: self.keccak_inputs.len() * keccak::keccak_stark::NUM_ROUNDS,
            keccak_sponge_len: self
                .keccak_sponge_ops
                .iter()
                .map(|op| op.input.len() / keccak_sponge::columns::KECCAK_RATE_BYTES + 1)
                .sum(),
            logic_len: self.logic_ops.len(),
            // This is technically a lower-bound, as we may fill gaps,
            // but this gives a relatively good estimate.
            memory_len: self.memory_ops.len(),
        }
    }

    /// Returns the number of operations for each STARK module.
    pub(crate) fn checkpoint(&self) -> TraceCheckpoint {
        TraceCheckpoint {
            arithmetic_len: self.arithmetic_ops.len(),
            byte_packing_len: self.byte_packing_ops.len(),
            cpu_len: self.cpu.len(),
            keccak_len: self.keccak_inputs.len(),
            keccak_sponge_len: self.keccak_sponge_ops.len(),
            logic_len: self.logic_ops.len(),
            memory_len: self.memory_ops.len(),
        }
    }

    pub(crate) fn rollback(&mut self, checkpoint: TraceCheckpoint) {
        self.arithmetic_ops.truncate(checkpoint.arithmetic_len);
        self.byte_packing_ops.truncate(checkpoint.byte_packing_len);
        self.cpu.truncate(checkpoint.cpu_len);
        self.keccak_inputs.truncate(checkpoint.keccak_len);
        self.keccak_sponge_ops
            .truncate(checkpoint.keccak_sponge_len);
        self.logic_ops.truncate(checkpoint.logic_len);
        self.memory_ops.truncate(checkpoint.memory_len);
    }

    pub(crate) fn mem_ops_since(&self, checkpoint: TraceCheckpoint) -> &[MemoryOp] {
        &self.memory_ops[checkpoint.memory_len..]
    }

    pub(crate) fn clock(&self) -> usize {
        self.cpu.len()
    }

    pub(crate) fn into_tables<const D: usize>(
        self,
        all_stark: &AllStark<T, D>,
        config: &StarkConfig,
        timing: &mut TimingTree,
    ) -> [Vec<PolynomialValues<T>>; NUM_TABLES]
    where
        T: RichField + Extendable<D>,
    {
        let cap_elements = config.fri_config.num_cap_elements();
        let Traces {
            arithmetic_ops,
            byte_packing_ops,
            cpu,
            logic_ops,
            memory_ops,
            keccak_inputs,
            keccak_sponge_ops,
        } = self;

        let arithmetic_trace = timed!(
            timing,
            log::Level::Info,
            "generate Arithmetic trace",
            all_stark.arithmetic_stark.generate_trace(arithmetic_ops)
        );
        let byte_packing_trace = timed!(
            timing,
            log::Level::Info,
            "generate BytePacking trace",
            all_stark
                .byte_packing_stark
                .generate_trace(byte_packing_ops, cap_elements, timing)
        );
        let cpu_rows = cpu.into_iter().map(|x| x.into()).collect();
        let cpu_trace = trace_rows_to_poly_values(cpu_rows);
        let keccak_trace = timed!(
            timing,
            log::Level::Info,
            "generate Keccak trace",
            all_stark
                .keccak_stark
                .generate_trace(keccak_inputs, cap_elements, timing)
        );
        let keccak_sponge_trace = timed!(
            timing,
            log::Level::Info,
            "generate Keccak sponge trace",
            all_stark
                .keccak_sponge_stark
                .generate_trace(keccak_sponge_ops, cap_elements, timing)
        );
        let logic_trace = timed!(
            timing,
            log::Level::Info,
            "generate Logic trace",
            all_stark
                .logic_stark
                .generate_trace(logic_ops, cap_elements, timing)
        );
        let memory_trace = timed!(
            timing,
            log::Level::Info,
            "generate Memory trace",
            all_stark.memory_stark.generate_trace(memory_ops, timing)
        );

        [
            arithmetic_trace,
            byte_packing_trace,
            cpu_trace,
            keccak_trace,
            keccak_sponge_trace,
            logic_trace,
            memory_trace,
        ]
    }
}

impl<T: Copy> Default for Traces<T> {
    fn default() -> Self {
        Self::new()
    }
}

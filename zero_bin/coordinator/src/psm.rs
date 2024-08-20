//! This module helps with creating the [ProverStateManager]

use std::env::{self, VarError};

use tracing::{error, info, warn};
use zero_bin_common::prover_state::{
    circuit::{Circuit, CircuitConfig, CircuitSize, NUM_TABLES},
    CircuitPersistence, ProverStateManager, TableLoadStrategy,
};

pub const PSM_CIRCUIT_TABLE_LOAD_STRAT_ENVKEY: &str = "PSM_TABLE_LOAD_STRAT";
pub const PSM_CIRCUIT_PERSISTENCE_ENVKEY: &str = "PSM_CIRCUIT_PERSISTENCE";

use core::str::FromStr;

pub fn load_psm_from_env() -> ProverStateManager {
    let tbl_load_strat = match env::var(PSM_CIRCUIT_TABLE_LOAD_STRAT_ENVKEY) {
        Ok(tls) if tls.contains("ON_DEMAND") => {
            info!("Loaded OnDemand TabeLoadStrategy from .env");
            Some(TableLoadStrategy::OnDemand)
        }
        Ok(tls) if tls.contains("MONOLITHIC") => {
            info!("Loaded Monolithic TabeLoadStrategy from .env");
            Some(TableLoadStrategy::Monolithic)
        }
        Ok(tls) => {
            error!("Unknown Table Load Strategy: {}", tls);
            panic!("Unknown Table Load Strategy: {}", tls);
        }
        Err(env::VarError::NotPresent) => {
            info!("Table Load Strategy not present in .env");
            None
        }
        Err(env::VarError::NotUnicode(os_str)) => {
            error!("Non-Unicode string for Table Load Strategy: `{:?}`", os_str);
            panic!("Non-Unicode string for Table Load Strategy: `{:?}`", os_str);
        }
    };

    let persistence = match env::var(PSM_CIRCUIT_PERSISTENCE_ENVKEY) {
        Ok(persistence) if persistence.contains("NONE") => {
            info!("Loaded `None` CircuitPersistence from .env");
            CircuitPersistence::None
        }
        Ok(persistence) if persistence.contains("DISK") => match tbl_load_strat {
            Some(tbl_load_strat) => {
                info!("Loaded `Disk` CircuitPersistence from .env");
                CircuitPersistence::Disk(tbl_load_strat)
            }
            None => {
                warn!("Table Load Strategy not specified, using default");
                CircuitPersistence::Disk(TableLoadStrategy::default())
            }
        },
        Ok(persistence) => {
            error!("Unable to determine circiut persistence: `{}`", persistence);
            panic!("Unable to determine circiut persistence: `{}`", persistence);
        }
        Err(env::VarError::NotPresent) => {
            warn!("No circuit persistence specified, using default. Will attempt to use table load strategy if provided.");

            match (tbl_load_strat, CircuitPersistence::default()) {
                // If given a tbl_load_strat and Circuit Persistence as a disk is the default, we go ahead
                // and modify the tablt load strategy
                (Some(tbl_load), CircuitPersistence::Disk(_)) => {
                    CircuitPersistence::Disk(tbl_load)
                },
                // Given the table load strategy and the default, warn we don't know how to
                // apply the table load strategy and return the default
                (Some(tbl_load), dflt) => {
                    warn!("Default circuit persistence is {:?}, unsure how to apply table load strategy ({:?})", dflt, tbl_load);
                    dflt
                }
                // Return None and the circuit persistence
                (None, dflt) => dflt,
            }
        }
        Err(env::VarError::NotUnicode(os_str)) => {
            error!("Non-Unicode circiut persistence: {:?}", os_str);
            panic!("Non-Unicode circiut persistence: {:?}", os_str);
        }
    };

    // Create the circuits
    info!("Creating default circuits (as baseline)");
    let mut circuits = CircuitConfig::default();
    for tbl in 0..NUM_TABLES {
        let circuit = Circuit::from(tbl);
        let circuit_envkey = circuit.as_env_key();
        match std::env::var(circuit_envkey) {
            Ok(item) => match CircuitSize::from_str(&item) {
                Ok(size) => {
                    info!("Modifying `{}` circuit to `{}`", circuit, size);
                    circuits.set_circuit_size(circuit, size)
                }
                Err(err) => panic!("Failed to parse Range: {}", err),
            },
            Err(VarError::NotPresent) => warn!("Missing circuit envkey: {}", circuit_envkey),
            Err(VarError::NotUnicode(os_string)) => warn!(
                "Invalid circuit envkey variable: `{}` -> {:?} ",
                circuit_envkey, os_string
            ),
        }
    }

    ProverStateManager {
        circuit_config: circuits,
        persistence,
    }
}

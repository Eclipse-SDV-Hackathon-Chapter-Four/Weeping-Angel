// SPDX-FileCopyrightText: Copyright (c) 2026 Contributors to the Eclipse Foundation
// SPDX-License-Identifier: Apache-2.0

//! Fault provider trait and SOVD fault types.

use std::collections::BTreeMap;

use async_trait::async_trait;

/// ISO 14229 diagnostic trouble code status bits.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FaultStatus {
    pub test_failed: bool,
    pub test_failed_this_operation_cycle: bool,
    pub pending_dtc: bool,
    pub confirmed_dtc: bool,
    pub test_not_completed_since_last_clear: bool,
    pub test_failed_since_last_clear: bool,
    pub test_not_completed_this_operation_cycle: bool,
    pub warning_indicator_requested: bool,
}

impl FaultStatus {
    #[must_use]
    pub fn mask(&self) -> u8 {
        u8::from(self.test_failed)
            | (u8::from(self.test_failed_this_operation_cycle) << 1)
            | (u8::from(self.pending_dtc) << 2)
            | (u8::from(self.confirmed_dtc) << 3)
            | (u8::from(self.test_not_completed_since_last_clear) << 4)
            | (u8::from(self.test_failed_since_last_clear) << 5)
            | (u8::from(self.test_not_completed_this_operation_cycle) << 6)
            | (u8::from(self.warning_indicator_requested) << 7)
    }
}

/// A diagnostic trouble code exposed through the SOVD faults collection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fault {
    pub code: String,
    pub display_code: String,
    pub scope: String,
    pub name: String,
    pub translation_id: String,
    pub severity: u32,
    pub status: FaultStatus,
    pub symptom: Option<String>,
    pub symptom_translation_id: Option<String>,
    pub schema: Option<String>,
    pub occurrence_counter: u32,
    pub aging_counter: u32,
    pub healing_counter: u32,
    pub first_occurrence: Option<String>,
    pub last_occurrence: Option<String>,
}

/// Errors that can occur when accessing faults.
#[derive(Debug, Clone, thiserror::Error)]
pub enum FaultError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("invalid request: {0}")]
    BadRequest(String),
    #[error("service unavailable: {0}")]
    Unavailable(String),
    #[error("internal error: {0}")]
    Internal(String),
}

/// A `Result` alias where the `Err` variant is [`FaultError`].
pub type FaultResult<T> = std::result::Result<T, FaultError>;

/// Accesses the faults collection of one SOVD entity.
#[async_trait]
pub trait FaultProvider: Send + Sync + 'static {
    async fn list(&self) -> FaultResult<Vec<Fault>>;

    async fn get(&self, fault_code: &str) -> FaultResult<(Fault, BTreeMap<String, String>)>;

    async fn clear_all(&self) -> FaultResult<()>;

    async fn clear(&self, fault_code: &str) -> FaultResult<()>;
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn status_mask_uses_iso_14229_bit_positions() {
        let status = FaultStatus {
            test_failed: true,
            confirmed_dtc: true,
            warning_indicator_requested: true,
            ..FaultStatus::default()
        };

        assert_eq!(status.mask(), 0x89);
    }
}

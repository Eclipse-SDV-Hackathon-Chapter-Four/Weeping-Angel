// Copyright (c) 2026 Matthias Knöfel
//
// This program and the accompanying materials are made available under
// the terms of the Eclipse Public License 2.0 which accompanies this
// distribution, and is available at https://www.eclipse.org/legal/epl-2.0/
//
// AI Disclosure: This file was mostly AI-generated.
//
// SPDX-License-Identifier: EPL-2.0 and CC0-1.0
// Assisted-by: Claude Opus 5.5
//! Serializable SOVD views of DFM faults (`SovdFault` itself has no serde).

use std::collections::BTreeMap;

use dfm_lib::sovd_fault_manager::{SovdEnvData, SovdFault};
use schemars::JsonSchema;
use serde::Serialize;

/// DTC status flags (ISO 14229 status byte semantics).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, JsonSchema)]
pub struct FaultStatus {
    pub test_failed: bool,
    pub test_failed_this_operation_cycle: bool,
    pub pending_dtc: bool,
    pub confirmed_dtc: bool,
    pub test_not_completed_since_last_clear: bool,
    pub test_failed_since_last_clear: bool,
    pub test_not_completed_this_operation_cycle: bool,
    pub warning_indicator_requested: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mask: Option<String>,
}

/// One DFM fault as exposed over SOVD.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct FaultView {
    pub code: String,
    pub display_code: String,
    pub scope: String,
    pub fault_name: String,
    pub severity: u32,
    pub status: FaultStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symptom: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurrence_counter: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aging_counter: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub healing_counter: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_occurrence: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_occurrence: Option<String>,
}

impl FaultView {
    /// A fault is active while its current test result is "failed".
    pub fn is_active(&self) -> bool {
        self.status.test_failed
    }
}

impl From<&SovdFault> for FaultView {
    fn from(f: &SovdFault) -> Self {
        let s = f.typed_status.clone().unwrap_or_default();
        Self {
            code: f.code.clone(),
            display_code: f.display_code.clone(),
            scope: f.scope.clone(),
            fault_name: f.fault_name.clone(),
            severity: f.severity,
            status: FaultStatus {
                test_failed: s.test_failed.unwrap_or(false),
                test_failed_this_operation_cycle: s
                    .test_failed_this_operation_cycle
                    .unwrap_or(false),
                pending_dtc: s.pending_dtc.unwrap_or(false),
                confirmed_dtc: s.confirmed_dtc.unwrap_or(false),
                test_not_completed_since_last_clear: s
                    .test_not_completed_since_last_clear
                    .unwrap_or(false),
                test_failed_since_last_clear: s.test_failed_since_last_clear.unwrap_or(false),
                test_not_completed_this_operation_cycle: s
                    .test_not_completed_this_operation_cycle
                    .unwrap_or(false),
                warning_indicator_requested: s.warning_indicator_requested.unwrap_or(false),
                mask: s.mask,
            },
            symptom: f.symptom.clone(),
            occurrence_counter: f.occurrence_counter,
            aging_counter: f.aging_counter,
            healing_counter: f.healing_counter,
            first_occurrence: f.first_occurrence.clone(),
            last_occurrence: f.last_occurrence.clone(),
        }
    }
}

/// Data value of the `faults` / `faults.active` resources.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct FaultList {
    pub items: Vec<FaultView>,
}

/// Data value of a per-fault `fault.<code>` resource.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct FaultDetail {
    pub fault: FaultView,
    /// Environment data snapshot stored by the DFM with the fault.
    pub environment_data: BTreeMap<String, String>,
}

impl FaultDetail {
    pub fn new(fault: &SovdFault, env: &SovdEnvData) -> Self {
        Self {
            fault: fault.into(),
            environment_data: env.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        }
    }
}

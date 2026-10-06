// SPDX-FileCopyrightText: Copyright (c) 2026 Contributors to the Eclipse Foundation
// SPDX-License-Identifier: Apache-2.0

//! SOVD fault resource models.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::Items;

/// Diagnostic trouble code status bits.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct FaultStatus {
    pub test_failed: bool,
    pub test_failed_this_operation_cycle: bool,
    pub pending_dtc: bool,
    pub confirmed_dtc: bool,
    pub test_not_completed_since_last_clear: bool,
    pub test_failed_since_last_clear: bool,
    pub test_not_completed_this_operation_cycle: bool,
    pub warning_indicator_requested: bool,
    pub mask: String,
}

/// A diagnostic trouble code exposed through an entity's faults collection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
pub struct Fault {
    pub code: String,
    pub display_code: String,
    pub scope: String,
    pub fault_name: String,
    pub fault_translation_id: String,
    pub severity: u32,
    pub status: FaultStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symptom: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symptom_translation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub occurrence_counter: u32,
    pub aging_counter: u32,
    pub healing_counter: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_occurrence: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_occurrence: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment_data: Option<BTreeMap<String, String>>,
}

/// Fault collection response.
pub type Faults = Items<Fault>;

// Copyright (c) 2026 Peter Ulbrich
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
pub mod guardian_config;
pub mod guardian_faults;
pub mod guardian_model;
pub mod guardian_reporting;
pub mod guardian_runtime;
pub mod guardian_uprotocol;

pub use guardian_config::GuardianConfig;
pub use guardian_model::{BatterySample, Detection, DetectionClass, DetectionLevel, Signal};
pub use guardian_runtime::GuardianRuntime;

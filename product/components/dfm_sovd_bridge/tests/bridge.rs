//! Bridge tests against an in-process fake of the DFM query API.

use std::collections::HashMap;

use dfm_lib::DfmQueryApi;
use dfm_lib::sovd_fault_manager::{Error, SovdEnvData, SovdFault, SovdFaultStatus};
use dfm_sovd_bridge::{DfmClient, build_component};
use opensovd_core::{DataError, DataFilter};

const PATH: &str = "battery_guardian";

fn fault(code: &str, failed: bool) -> SovdFault {
    SovdFault {
        code: code.into(),
        display_code: code.into(),
        fault_name: code.into(),
        severity: 2,
        typed_status: Some(SovdFaultStatus {
            test_failed: Some(failed),
            confirmed_dtc: Some(failed),
            mask: Some(if failed { "0x09" } else { "0x00" }.into()),
            ..Default::default()
        }),
        occurrence_counter: Some(u32::from(failed)),
        ..Default::default()
    }
}

struct FakeDfm(Vec<SovdFault>);

impl DfmQueryApi for FakeDfm {
    fn get_all_faults(&self, path: &str) -> Result<Vec<SovdFault>, Error> {
        if path == PATH {
            Ok(self.0.clone())
        } else {
            Err(Error::BadArgument)
        }
    }

    fn get_fault(&self, path: &str, code: &str) -> Result<(SovdFault, SovdEnvData), Error> {
        let f = self
            .get_all_faults(path)?
            .into_iter()
            .find(|f| f.code == code)
            .ok_or(Error::NotFound)?;
        let env = HashMap::from([("temp_max".to_owned(), "71.5".to_owned())]);
        Ok((f, env))
    }

    fn delete_all_faults(&self, _path: &str) -> Result<(), Error> {
        Ok(())
    }

    fn delete_fault(&self, _path: &str, _code: &str) -> Result<(), Error> {
        Ok(())
    }
}

fn client() -> DfmClient {
    DfmClient::spawn(|| {
        Ok(FakeDfm(vec![
            fault("BatteryOverTempCritical", true),
            fault("BatterySocRange", false),
        ]))
    })
    .unwrap()
}

#[tokio::test]
async fn exposes_aggregate_and_per_fault_resources() {
    let dfm = client();
    let codes = dfm
        .all_faults(PATH)
        .await
        .unwrap()
        .into_iter()
        .map(|f| f.code)
        .collect::<Vec<_>>();
    let component = build_component(&dfm, PATH, "Battery Guardian", &codes).unwrap();
    let provider = component.data_provider().unwrap();

    let mut ids: Vec<_> = provider
        .list(DataFilter::default())
        .await
        .unwrap()
        .into_iter()
        .map(|m| m.id)
        .collect();
    ids.sort();
    assert_eq!(
        ids,
        [
            "fault.BatteryOverTempCritical",
            "fault.BatterySocRange",
            "faults",
            "faults.active",
        ]
    );
}

#[tokio::test]
async fn active_list_contains_only_failed_faults() {
    let dfm = client();
    let component = build_component(&dfm, PATH, "Battery Guardian", &[]).unwrap();
    let provider = component.data_provider().unwrap();

    let all = provider.read("faults", false).await.unwrap().data;
    assert_eq!(all["items"].as_array().unwrap().len(), 2);

    let active = provider.read("faults.active", false).await.unwrap().data;
    let items = active["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["code"], "BatteryOverTempCritical");
    assert_eq!(items[0]["status"]["test_failed"], true);
    assert_eq!(items[0]["status"]["mask"], "0x09");
}

#[tokio::test]
async fn fault_detail_carries_environment_data() {
    let dfm = client();
    let codes = vec!["BatteryOverTempCritical".to_owned(), "Unknown".to_owned()];
    let component = build_component(&dfm, PATH, "Battery Guardian", &codes).unwrap();
    let provider = component.data_provider().unwrap();

    let detail = provider
        .read("fault.BatteryOverTempCritical", false)
        .await
        .unwrap()
        .data;
    assert_eq!(detail["fault"]["occurrence_counter"], 1);
    assert_eq!(detail["environment_data"]["temp_max"], "71.5");

    let missing = provider.read("fault.Unknown", false).await;
    assert!(matches!(missing, Err(DataError::NotFound(_))));
}

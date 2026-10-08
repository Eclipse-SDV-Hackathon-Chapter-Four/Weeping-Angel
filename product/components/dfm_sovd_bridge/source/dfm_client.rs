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
//! Async front end for the DFM query IPC (`dfm/query`, iceoryx2).
//!
//! The iceoryx2 client blocks while polling for a response, so it lives on a
//! dedicated worker thread; SOVD handlers send jobs and await the reply.

use std::sync::mpsc;
use std::thread;

use dfm_lib::DfmQueryApi;
use dfm_lib::sovd_fault_manager::{Error, SovdEnvData, SovdFault};
use tokio::sync::oneshot;

type Job = Box<dyn FnOnce(&dyn DfmQueryApi) + Send>;

/// Cloneable handle to the DFM query worker.
#[derive(Clone)]
pub struct DfmClient {
    jobs: mpsc::Sender<Job>,
}

impl DfmClient {
    /// Spawn the worker; `connect` runs on the worker thread and creates the
    /// query client there (e.g. `Iceoryx2DfmQuery::with_timeout`).
    pub fn spawn<A, F>(connect: F) -> anyhow::Result<Self>
    where
        A: DfmQueryApi + 'static,
        F: FnOnce() -> Result<A, Error> + Send + 'static,
    {
        let (jobs, rx) = mpsc::channel::<Job>();
        let (ready_tx, ready_rx) = mpsc::channel();
        thread::Builder::new()
            .name("dfm-query".into())
            .spawn(move || {
                let api = match connect() {
                    Ok(api) => {
                        let _ = ready_tx.send(Ok(()));
                        api
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                for job in rx {
                    job(&api);
                }
            })?;
        ready_rx
            .recv()?
            .map_err(|e| anyhow::anyhow!("cannot open DFM query service: {e}"))?;
        Ok(Self { jobs })
    }

    async fn call<T, F>(&self, f: F) -> Result<T, Error>
    where
        T: Send + 'static,
        F: FnOnce(&dyn DfmQueryApi) -> Result<T, Error> + Send + 'static,
    {
        let (tx, rx) = oneshot::channel();
        self.jobs
            .send(Box::new(move |api| {
                let _ = tx.send(f(api));
            }))
            .map_err(|_| Error::Storage("DFM query worker stopped".into()))?;
        rx.await
            .map_err(|_| Error::Storage("DFM query worker dropped request".into()))?
    }

    pub async fn all_faults(&self, path: &str) -> Result<Vec<SovdFault>, Error> {
        let path = path.to_owned();
        self.call(move |api| api.get_all_faults(&path)).await
    }

    pub async fn fault(&self, path: &str, code: &str) -> Result<(SovdFault, SovdEnvData), Error> {
        let (path, code) = (path.to_owned(), code.to_owned());
        self.call(move |api| api.get_fault(&path, &code)).await
    }
}

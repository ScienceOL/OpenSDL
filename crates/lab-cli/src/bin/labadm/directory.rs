use std::net::SocketAddr;

use osdl_proto::etcd::{
    compare::{Operand, Target},
    request_op, Compare, DeleteRangeRequest, KeyValue, PutRequest, RangeRequest, RequestOp,
    TxnRequest,
};
use tonic::Status;

use crate::{
    config::{worker_address, Id},
    etcd::Etcd,
};

const SCHEMA: &[u8] = b"1";
pub const PAGE_SIZE: usize = 8;

#[derive(Clone)]
pub struct Directory {
    etcd: Etcd,
    marker: Vec<u8>,
    worker_prefix: String,
}

#[derive(Clone)]
pub struct Worker {
    pub id: Id,
    pub address: SocketAddr,
}

pub struct Page {
    pub workers: Vec<Worker>,
    pub next_after: String,
}

impl Directory {
    pub fn new(etcd: Etcd, cluster: &Id) -> Self {
        Self {
            etcd,
            marker: format!("/opensdl/v1/{cluster}/schema").into_bytes(),
            worker_prefix: format!("/opensdl/v1/{cluster}/workers/"),
        }
    }

    pub async fn initialize(&self) -> Result<(), Status> {
        let key = self.marker.clone();
        if self
            .write(
                vec![absent(key.clone())],
                request_op::Request::Put(PutRequest {
                    key,
                    value: SCHEMA.to_vec(),
                }),
            )
            .await?
        {
            return Ok(());
        }
        // Includes a retry whose first transaction committed.
        self.ready().await
    }

    pub async fn ready(&self) -> Result<(), Status> {
        match self.get(self.marker.clone()).await? {
            Some(kv) if kv.value == SCHEMA => Ok(()),
            _ => Err(Status::failed_precondition(
                "cluster is not initialized with this schema",
            )),
        }
    }

    pub async fn admit(&self, worker: &Worker) -> Result<(), Status> {
        let key = self.worker_key(&worker.id);
        let value = worker.address.to_string().into_bytes();
        let marker = Compare {
            target: Target::Value.into(),
            key: self.marker.clone(),
            operand: Some(Operand::Value(SCHEMA.to_vec())),
        };
        let put = request_op::Request::Put(PutRequest {
            key: key.clone(),
            value: value.clone(),
        });
        if self.write(vec![marker, absent(key.clone())], put).await? {
            return Ok(());
        }
        self.ready().await?;
        match self.get(key).await? {
            Some(kv) if kv.value == value => Ok(()),
            _ => Err(Status::already_exists(
                "worker ID already has a different route",
            )),
        }
    }

    pub async fn forget(&self, id: &Id) -> Result<(), Status> {
        let key = self.worker_key(id);
        let Some(worker) = self.get(key.clone()).await? else {
            return Ok(());
        };
        let revision = Compare {
            target: Target::Mod.into(),
            key: key.clone(),
            operand: Some(Operand::ModRevision(worker.mod_revision)),
        };
        let delete = request_op::Request::DeleteRange(DeleteRangeRequest { key: key.clone() });
        if self.write(vec![revision], delete).await? || self.get(key).await?.is_none() {
            return Ok(());
        }
        Err(Status::aborted(
            "worker registration changed during removal",
        ))
    }

    pub async fn worker(&self, id: &Id) -> Result<Worker, Status> {
        let entry = self
            .get(self.worker_key(id))
            .await?
            .ok_or_else(|| Status::not_found(format!("unknown worker {id}")))?;
        self.decode_worker(entry)
    }

    pub async fn page(&self, after: Option<Id>) -> Result<Page, Status> {
        // The byte immediately after '/' ends the prefix range. etcd's default
        // key order is ascending and can apply the limit at the storage layer.
        let range_end = format!("{}0", self.worker_prefix.trim_end_matches('/')).into_bytes();
        let key = match after {
            Some(id) => {
                let mut key = self.worker_key(&id);
                key.push(0);
                key
            }
            None => self.worker_prefix.as_bytes().to_vec(),
        };
        let page = self
            .etcd
            .range(RangeRequest {
                key,
                range_end,
                limit: PAGE_SIZE as i64,
            })
            .await?;
        let workers = page
            .kvs
            .into_iter()
            .map(|entry| self.decode_worker(entry))
            .collect::<Result<Vec<_>, _>>()?;
        let next_after = workers
            .last()
            .filter(|_| page.more)
            .map(|worker| worker.id.to_string())
            .unwrap_or_default();
        Ok(Page {
            workers,
            next_after,
        })
    }

    async fn get(&self, key: Vec<u8>) -> Result<Option<KeyValue>, Status> {
        let response = self
            .etcd
            .range(RangeRequest {
                key,
                limit: 1,
                ..Default::default()
            })
            .await?;
        Ok(response.kvs.into_iter().next())
    }

    async fn write(
        &self,
        compare: Vec<Compare>,
        request: request_op::Request,
    ) -> Result<bool, Status> {
        self.etcd
            .txn(TxnRequest {
                compare,
                success: vec![RequestOp {
                    request: Some(request),
                }],
            })
            .await
    }

    fn worker_key(&self, id: &Id) -> Vec<u8> {
        format!("{}{id}", self.worker_prefix).into_bytes()
    }

    fn decode_worker(&self, record: KeyValue) -> Result<Worker, Status> {
        let invalid = || Status::data_loss("invalid worker route in cluster directory");
        let id = std::str::from_utf8(&record.key)
            .map_err(|_| invalid())?
            .strip_prefix(&self.worker_prefix)
            .ok_or_else(invalid)?
            .parse()
            .map_err(|_| invalid())?;
        let address = std::str::from_utf8(&record.value)
            .map_err(|_| invalid())?
            .parse()
            .map_err(|_| invalid())?;
        Ok(Worker {
            id,
            address: worker_address(address).map_err(|_| invalid())?,
        })
    }
}

fn absent(key: Vec<u8>) -> Compare {
    Compare {
        target: Target::Version.into(),
        key,
        operand: Some(Operand::Version(0)),
    }
}

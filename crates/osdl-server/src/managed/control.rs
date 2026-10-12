use super::store::Record;
use anyhow::{bail, Context};
use osdl_proto::v1::{self as pb, osdl_client::OsdlClient};
use tonic::{
    transport::{Channel, Endpoint},
    Request,
};

pub(crate) type Client = OsdlClient<Channel>;
pub(crate) fn request<T>(body: T, token: &str) -> anyhow::Result<Request<T>> {
    let mut request = Request::new(body);
    request
        .metadata_mut()
        .insert("authorization", format!("Bearer {token}").parse()?);
    Ok(request)
}
pub(crate) async fn connect(record: &Record) -> anyhow::Result<Client> {
    Ok(verified_status(record).await?.0)
}
pub(crate) async fn ready(record: &Record) -> anyhow::Result<bool> {
    Ok(verified_status(record)
        .await?
        .1
        .engine
        .is_some_and(|engine| engine.device_count > 0))
}
async fn verified_status(record: &Record) -> anyhow::Result<(Client, pb::StatusResponse)> {
    let endpoint = record
        .info
        .endpoint
        .as_deref()
        .context("instance has no live endpoint")?;
    let channel = Endpoint::from_shared(endpoint.to_owned())?
        .connect_timeout(std::time::Duration::from_secs(2))
        .timeout(std::time::Duration::from_secs(2))
        .connect()
        .await?;
    let mut client = OsdlClient::new(channel);
    let response = client
        .status(request(pb::StatusRequest {}, &record.token)?)
        .await?
        .into_inner();
    if response.instance != record.info.name || Some(response.pid) != record.info.pid {
        bail!("managed process identity does not match");
    }
    Ok((client, response))
}

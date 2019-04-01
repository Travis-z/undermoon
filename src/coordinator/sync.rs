use super::broker::MetaDataBroker;
use super::core::{CoordinateError, HostMetaRetriever, HostMetaSender};
use common::cluster::{Host, SlotRange};
use common::db::{DBMapFlags, HostDBMap};
use futures::{future, Future};
use protocol::{RedisClient, Resp};
use std::collections::HashMap;

pub struct HostMetaRespSender<C: RedisClient> {
    client: C,
}

impl<C: RedisClient> HostMetaRespSender<C> {
    pub fn new(client: C) -> Self {
        Self { client }
    }
}

impl<C: RedisClient> HostMetaSender for HostMetaRespSender<C> {
    fn send_meta(&self, host: Host) -> Box<dyn Future<Item = (), Error = CoordinateError> + Send> {
        Box::new(send_meta(
            &self.client,
            host,
            "SETDB".to_string(),
            DBMapFlags { force: false },
        ))
    }
}

pub struct PeerMetaRespSender<C: RedisClient> {
    client: C,
}

impl<C: RedisClient> PeerMetaRespSender<C> {
    pub fn new(client: C) -> Self {
        Self { client }
    }
}

impl<C: RedisClient> HostMetaSender for PeerMetaRespSender<C> {
    fn send_meta(&self, host: Host) -> Box<dyn Future<Item = (), Error = CoordinateError> + Send> {
        Box::new(send_meta(
            &self.client,
            host,
            "SETPEER".to_string(),
            DBMapFlags { force: false },
        ))
    }
}

pub struct LocalMetaRetriever<B: MetaDataBroker> {
    broker: B,
}

impl<B: MetaDataBroker> LocalMetaRetriever<B> {
    pub fn new(broker: B) -> Self {
        Self { broker }
    }
}

impl<B: MetaDataBroker> HostMetaRetriever for LocalMetaRetriever<B> {
    fn get_host_meta(
        &self,
        address: String,
    ) -> Box<dyn Future<Item = Option<Host>, Error = CoordinateError> + Send> {
        Box::new(
            self.broker
                .get_host(address)
                .map_err(CoordinateError::MetaData),
        )
    }
}

pub struct PeerMetaRetriever<B: MetaDataBroker> {
    broker: B,
}

impl<B: MetaDataBroker> PeerMetaRetriever<B> {
    pub fn new(broker: B) -> Self {
        Self { broker }
    }
}

impl<B: MetaDataBroker> HostMetaRetriever for PeerMetaRetriever<B> {
    fn get_host_meta(
        &self,
        address: String,
    ) -> Box<dyn Future<Item = Option<Host>, Error = CoordinateError> + Send> {
        Box::new(
            self.broker
                .get_peer(address)
                .map_err(CoordinateError::MetaData),
        )
    }
}

// sub_command should be SETDB or SETPEER
fn send_meta<C: RedisClient>(
    client: &C,
    host: Host,
    sub_command: String,
    flags: DBMapFlags,
) -> impl Future<Item = (), Error = CoordinateError> + Send {
    let address = host.get_address().clone();
    let epoch = host.get_epoch();
    let mut db_map: HashMap<String, HashMap<String, Vec<SlotRange>>> = HashMap::new();
    for node in host.get_nodes() {
        let dbs = db_map
            .entry(node.get_cluster_name().clone())
            .or_insert_with(HashMap::new);
        dbs.insert(node.get_address().clone(), node.get_slots().clone());
    }
    let args = HostDBMap::new(epoch, flags.clone(), db_map).db_map_to_args();
    let mut cmd = vec![
        "UMCTL".to_string(),
        sub_command.clone(),
        epoch.to_string(),
        flags.to_arg(),
    ];
    cmd.extend(args.into_iter());
    debug!("sending meta {} {:?}", sub_command, cmd);
    client
        .execute(address, cmd.into_iter().map(|s| s.into_bytes()).collect())
        .map_err(|e| {
            error!("failed to send meta data of host {:?}", e);
            CoordinateError::Redis(e)
        })
        .and_then(move |resp| match resp {
            Resp::Error(err_str) => {
                error!("failed to send meta, invalid reply {:?}", err_str);
                future::err(CoordinateError::InvalidReply)
            }
            reply => {
                debug!("Successfully set meta {} {:?}", sub_command, reply);
                future::ok(())
            }
        })
}
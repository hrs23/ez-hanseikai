use std::sync::Arc;
use std::time::Duration;

use iroh::endpoint::{default_relay_mode, Connection};
use iroh::{Endpoint, RelayConfig, RelayUrl, TransportAddr, Watcher};
use tokio::time::{sleep, Instant};

use crate::{debug, Core};

const RELAY_EVERY: Duration = Duration::from_secs(300);
const RELAY_WAIT: Duration = Duration::from_secs(10);
const RELAY_POLL: Duration = Duration::from_millis(500);

pub fn url(u: &str) -> Option<RelayUrl> {
    u.trim().parse::<RelayUrl>().ok().filter(|r| r.scheme() == "https" && r.host_str().is_some())
}

pub fn parse(urls: &[String]) -> Result<Vec<RelayUrl>, String> {
    urls.iter().map(|u| u.trim()).filter(|u| !u.is_empty()).map(|u| url(u).ok_or(format!("Not a relay URL: {u}"))).collect()
}

fn public() -> Vec<RelayUrl> {
    let mut urls: Vec<RelayUrl> = default_relay_mode().relay_map().urls();
    urls.sort();
    urls
}

pub fn groups(custom: &[RelayUrl]) -> Vec<Vec<RelayUrl>> {
    custom.iter().map(|u| vec![u.clone()]).chain([public()]).collect()
}

pub fn home(core: &Core) -> Option<RelayUrl> {
    core.ep.home_relay_status().get().into_iter().find(|s| s.is_connected()).map(|s| s.url().clone())
}

pub fn kind(addr: &TransportAddr) -> &'static str {
    match addr {
        TransportAddr::Relay(_) => "relay",
        TransportAddr::Ip(_) => "direct",
        _ => "other",
    }
}

pub fn path_kind(conn: &Connection) -> (&'static str, u64, Option<String>) {
    let paths = conn.paths();
    paths.iter().find(|p| p.is_selected()).or_else(|| paths.iter().next()).map_or(("none", 0, None), |p| {
        let host = match p.remote_addr() {
            TransportAddr::Relay(url) => url.host_str().map(String::from),
            _ => None,
        };
        (kind(p.remote_addr()), p.rtt().as_millis() as u64, host)
    })
}

pub fn relayed(conn: &Connection) -> bool {
    path_kind(conn).0 == "relay"
}

async fn measured(ep: &Endpoint, url: &RelayUrl) -> bool {
    let until = Instant::now() + RELAY_WAIT;
    while Instant::now() < until {
        if ep.net_report().get().is_some_and(|r| r.relay_latency.iter().any(|(_, u, _)| u == url)) {
            return true;
        }
        sleep(RELAY_POLL).await;
    }
    false
}

async fn insert(ep: &Endpoint, map: &mut Vec<RelayUrl>, url: &RelayUrl) -> bool {
    let added = !map.contains(url);
    if added {
        ep.insert_relay(url.clone(), Arc::new(RelayConfig::from(url.clone()))).await;
        map.push(url.clone());
    }
    added
}

async fn alive(ep: &Endpoint, map: &mut Vec<RelayUrl>, url: &RelayUrl) -> bool {
    let added = insert(ep, map, url).await;
    let ok = measured(ep, url).await;
    if added && !ok {
        ep.remove_relay(url).await;
        map.pop();
    }
    ok
}

async fn choose(ep: &Endpoint, map: &mut Vec<RelayUrl>, group: &[RelayUrl]) {
    for url in group {
        insert(ep, map, url).await;
    }
    for url in map.iter().filter(|u| !group.contains(u)) {
        ep.remove_relay(url).await;
    }
    *map = group.to_vec();
}

pub async fn keep(core: Arc<Core>) {
    let mut map = public();
    loop {
        let candidates = groups(&core.relays.lock().unwrap().clone());
        let last = candidates.len() - 1;
        for (i, group) in candidates.iter().enumerate() {
            if *group == map {
                if home(&core).is_some_and(|h| map.contains(&h)) {
                    break;
                }
                continue;
            }
            if i == last || alive(&core.ep, &mut map, &group[0]).await {
                choose(&core.ep, &mut map, group).await;
                debug::event(&core, std::time::Instant::now(), if i == last { "relay: the public relays".into() } else { format!("relay: server {} of the list", i + 1) });
                break;
            }
        }
        tokio::select! {
            _ = sleep(RELAY_EVERY) => {}
            _ = core.relay_wake.notified() => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_relays_come_first_each_alone_and_the_public_relays_last_together() {
        let custom = parse(&["https://relay.example.org/".into(), " ".into(), "https://second.example.net".into()]).unwrap();
        let list = groups(&custom);
        assert_eq!(list.len(), 3);
        assert_eq!((list[0].len(), list[0][0].host_str(), list[1][0].host_str()), (1, Some("relay.example.org"), Some("second.example.net")));
        assert_eq!(list[2], public());
        assert!(list[2].len() > 1 && !list[2].contains(&custom[0]));
        assert_eq!(groups(&[]), vec![public()]);
        assert_eq!(parse(&["http://relay.example.org".into()]), Err("Not a relay URL: http://relay.example.org".into()));
        assert_eq!(parse(&["relay.example.org".into()]), Err("Not a relay URL: relay.example.org".into()));
    }
}

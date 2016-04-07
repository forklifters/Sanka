use tracker::Tracker;
use response::TrackerResponse;
use error::ErrorResponse;
use announce::{Action, Announce};

use hyper::server::{Request, Response, Handler};
use hyper::uri::RequestUri::AbsolutePath;
use std::net::{IpAddr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::collections::HashMap;
use std::sync::Arc;
use url::form_urlencoded;
use std::str::FromStr;

pub struct RequestHandler {
    pub tracker: Arc<Tracker>,
}

impl Handler for RequestHandler {
    fn handle(&self, req: Request, res: Response) {
        let resp = match req.uri {
            AbsolutePath(ref path) => {
                match path.find('?') {
                    Some(i) => {
                        let (action, param_str) = path.split_at(i + 1);
                        let param_vec = form_urlencoded::parse(param_str.as_bytes());
                        match action {
                            "/announce?" => {
                                let announce = (request_to_announce(&req, param_vec)).unwrap();
                                self.tracker.handle_announce(announce)
                            }
                            "/scrape?" => self.tracker.handle_scrape(param_vec),
                            _ => Err(ErrorResponse::BadAction),
                        }
                    }
                    None => Err(ErrorResponse::BadRequest),
                }
            }
            _ => Err(ErrorResponse::BadAction),
        };
        res.send(bencode_result(resp).as_slice()).unwrap();
    }
}

fn request_to_announce(req: &Request, param_vec: Vec<(String, String)>) -> Result<Announce, ErrorResponse>
{
    let mut params = HashMap::new();
    for (key, val) in param_vec {
        params.insert(key, val);
    }

    let info_hash: String = try!(get_from_params(&params, String::from("info_hash")));
    let pid = try!(get_from_params(&params, String::from("peer_id")));
    let ul = try!(get_from_params(&params, String::from("uploaded")));
    let dl = try!(get_from_params(&params, String::from("downloaded")));
    let left = try!(get_from_params(&params, String::from("left")));

    // IP parsing according to BEP 0007 with additional proxy forwarding check
    let port = try!(get_from_params(&params, String::from("port")));
    let (ipv4, ipv6) = get_ips(&params, req, &port);
    let action = match get_from_params::<String>(&params, String::from("event")) {
        Ok(ev_str) => {
            match &ev_str[..] {
                "started" => get_action(left),
                "stopped" => Action::Stopped,
                "completed" => Action::Completed,
                _ => get_action(left),
            }
        }
        Err(_) => get_action(left),
    };

    let numwant = match get_from_params::<u8>(&params, String::from("numwant")) {
        Ok(amount) => {
            if amount > 25 {
                25
            } else {
                amount
            }
        }
        Err(_) => 25,
    };

    Ok(Announce {
        info_hash: info_hash,
        peer_id: pid,
        ipv4: ipv4,
        ipv6: ipv6,
        ul: ul,
        dl: dl,
        left: left,
        action: action,
        numwant: numwant,
    })
}

fn get_ips(params: &HashMap<String, String>,
           req: &Request,
           port: &u16)
           -> (Option<SocketAddrV4>, Option<SocketAddrV6>) {
    let port = *port;
    let default_ip = match req.headers.get_raw("X-Forwarded-For") {
        Some(bytes) => {
            match String::from_utf8(bytes[0].clone()) {
                Ok(ip_str) => {
                    match ip_str.parse::<IpAddr>() {
                        Ok(ip) => SocketAddr::new(ip, port),
                        Err(_) => req.remote_addr,
                    }
                }
                Err(_) => req.remote_addr,
            }
        }
        None => req.remote_addr,
    };
    let ip = match get_from_params(&params, String::from("ip")) {
        Ok(ip) => SocketAddr::new(ip, port),
        Err(_) => default_ip,
    };

    match ip {
        SocketAddr::V4(v4) => {
            let v6 = match get_socket(&params, String::from("ipv6"), port) {
                Some(sock) => {
                    match sock {
                        SocketAddr::V6(v6) => Some(v6),
                        _ => None,
                    }
                }
                None => None,
            };
            (Some(v4), v6)
        }
        SocketAddr::V6(v6) => {
            let v4 = match get_socket(&params, String::from("ipv4"), port) {
                Some(sock) => {
                    match sock {
                        SocketAddr::V4(v4) => Some(v4),
                        _ => None,
                    }
                }
                None => None,
            };
            (v4, Some(v6))
        }
    }
}

fn get_from_params<T: FromStr>(map: &HashMap<String, String>,
                               key: String)
                               -> Result<T, ErrorResponse> {
    match map.get(&key) {
        Some(res) => {
            match res.parse::<T>() {
                Ok(val) => Ok(val),
                Err(_) => Err(ErrorResponse::BadRequest),
            }
        }
        None => Err(ErrorResponse::BadRequest),
    }
}

fn get_socket(params: &HashMap<String, String>, key: String, port: u16) -> Option<SocketAddr> {
    let ip: Result<IpAddr, ErrorResponse> = get_from_params(params, key.clone());
    let socket: Result<SocketAddr, ErrorResponse> = get_from_params(params, key.clone());
    match (ip, socket) {
        (Err(_), Err(_)) => None,
        (Ok(ip), Err(_)) => Some(SocketAddr::new(ip, port)),
        (Err(_), Ok(sock)) => Some(sock),
        _ => None,
    }
}

fn get_action(left: u64) -> Action {
    if left == 0 {
        Action::Seeding
    } else {
        Action::Leeching
    }
}

fn bencode_result<S: TrackerResponse, E: TrackerResponse>(result: Result<S, E>) -> Vec<u8> {
    match result {
        Ok(resp) => resp.to_bencode(),
        Err(err) => err.to_bencode(),
    }
}

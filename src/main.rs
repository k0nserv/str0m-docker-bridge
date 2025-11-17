#[macro_use]
extern crate tracing;

use std::io::ErrorKind;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::process;
use std::sync::Arc;
use std::thread;
use std::time::Instant;

use rouille::Server;
use rouille::{Request, Response};

use str0m::change::SdpOffer;
use str0m::config::CryptoProvider;
use str0m::net::Protocol;
use str0m::net::Receive;
use str0m::{Candidate, Event, IceConnectionState, Input, Output, Rtc, RtcConfig, RtcError};

mod util;

fn init_log() {
    use tracing_subscriber::{fmt, prelude::*, EnvFilter};

    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("str0m_docker=debug,str0m=debug,dimpl=debug"));

    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(env_filter)
        .init();
}

/// Parse environment variables for Docker bridge mode configuration
fn parse_docker_config() -> Option<DockerConfig> {
    let public_ip = std::env::var("PUBLIC_IP").ok()?;
    let public_ip: IpAddr = public_ip.parse().ok()?;

    let bind_ip = std::env::var("BIND_IP")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| "0.0.0.0".parse().unwrap());

    Some(DockerConfig { bind_ip, public_ip })
}

#[derive(Debug, Clone, Copy)]
struct DockerConfig {
    bind_ip: IpAddr,
    public_ip: IpAddr,
}

pub fn main() {
    init_log();

    // Run with whatever is configured.
    CryptoProvider::from_feature_flags().install_process_default();

    let certificate = include_bytes!("../cer.pem").to_vec();
    let private_key = include_bytes!("../key.pem").to_vec();

    let docker_config = parse_docker_config();

    if let Some(ref config) = docker_config {
        info!(
            "Running in Docker bridge mode: bind={}, public={}",
            config.bind_ip, config.public_ip
        );
    } else {
        info!("Running in standard mode (no PUBLIC_IP set)");
    }
    let (socket, candidate_addr) = if let Some(config) = docker_config {
        // Docker bridge mode: bind to BIND_IP, advertise PUBLIC_IP
        let bind_addr = format!("{}:10000", config.bind_ip);
        info!("Binding UDP socket to {}", bind_addr);

        let socket = UdpSocket::bind(&bind_addr).expect("binding a random UDP port");
        let local_addr = socket.local_addr().expect("a local socket address");

        // Create candidate with public IP but the port we actually bound to
        let candidate_addr = SocketAddr::new(config.public_ip, local_addr.port());

        info!(
            "Socket bound to {}, advertising candidate {}",
            local_addr, candidate_addr
        );

        (socket, candidate_addr)
    } else {
        // Standard mode: auto-detect host address
        let addr = util::select_host_address();
        let bind_addr = format!("{}:0", addr);

        let socket = UdpSocket::bind(&bind_addr).expect("binding a random UDP port");
        let addr = socket.local_addr().expect("a local socket address");

        info!("Standard mode: socket bound to {}", addr);

        (socket, addr)
    };
    let socket = Arc::new(socket);

    let server = Server::new_ssl(
        "0.0.0.0:3000",
        move |request| web_request(request, socket.clone(), candidate_addr),
        certificate,
        private_key,
    )
    .expect("starting the web server");

    let port = server.server_addr().port();

    if let Some(config) = docker_config {
        info!("Connect a browser to https://{}:{}", config.public_ip, port);
    } else {
        let host_addr = util::select_host_address();
        info!("Connect a browser to https://{}:{}", host_addr, port);
    }

    server.run();
}

// Handle a web request.
fn web_request(request: &Request, socket: Arc<UdpSocket>, candidate_addr: SocketAddr) -> Response {
    if request.method() == "GET" {
        return Response::html(include_str!("../http-post.html"));
    }

    // Expected POST SDP Offers.
    let mut data = request.data().expect("body to be available");

    let offer: SdpOffer = serde_json::from_reader(&mut data).expect("serialized offer");
    let mut rtc = RtcConfig::new().set_ice_lite(true).build();

    let candidate = Candidate::host(candidate_addr, "udp").expect("a host candidate");
    rtc.add_local_candidate(candidate).unwrap();

    // Create an SDP Answer.
    let answer = rtc
        .sdp_api()
        .accept_offer(offer)
        .expect("offer to be accepted");

    // Launch WebRTC in separate thread.
    thread::spawn(move || {
        if let Err(e) = run(rtc, socket, candidate_addr) {
            eprintln!("Exited: {e:?}");
            process::exit(1);
        }
    });

    let body = serde_json::to_vec(&answer).expect("answer to serialize");

    Response::from_data("application/json", body)
}

fn run(mut rtc: Rtc, socket: Arc<UdpSocket>, candidate_addr: SocketAddr) -> Result<(), RtcError> {
    // Buffer for incoming data.
    let mut buf = Vec::new();

    loop {
        // Poll output until we get a timeout. The timeout means we are either awaiting UDP socket input
        // or the timeout to happen.
        let timeout = match rtc.poll_output()? {
            Output::Timeout(v) => v,

            Output::Transmit(v) => {
                socket.send_to(&v.contents, v.destination)?;
                continue;
            }

            Output::Event(v) => {
                if v == Event::IceConnectionStateChange(IceConnectionState::Disconnected) {
                    return Ok(());
                }
                continue;
            }
        };

        let timeout = timeout - Instant::now();

        // socket.set_read_timeout(Some(0)) is not ok
        if timeout.is_zero() {
            rtc.handle_input(Input::Timeout(Instant::now()))?;
            continue;
        }

        socket.set_read_timeout(Some(timeout))?;
        buf.resize(2000, 0);

        let input = match socket.recv_from(&mut buf) {
            Ok((n, source)) => {
                dbg!(n, source);
                buf.truncate(n);
                Input::Receive(
                    Instant::now(),
                    Receive {
                        proto: Protocol::Udp,
                        source,
                        destination: candidate_addr,
                        contents: buf.as_slice().try_into()?,
                    },
                )
            }

            Err(e) => match e.kind() {
                // Expected error for set_read_timeout(). One for windows, one for the rest.
                ErrorKind::WouldBlock | ErrorKind::TimedOut => Input::Timeout(Instant::now()),
                _ => return Err(e.into()),
            },
        };

        rtc.handle_input(input)?;
    }
}

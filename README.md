# Running str0m in Docker Bridge Mode

This document explains how to run the str0m WebRTC application in Docker using bridge networking mode.

## The Challenge: WebRTC + Docker + Bridge Networking

WebRTC requires clients to connect directly to the server's IP address for media streaming. When running in Docker with bridge mode networking:

- **Container has a private IP** (e.g., 172.17.0.x) that's not accessible from outside
- **Host has a public IP** that clients can reach
- **Port forwarding** maps host ports to container ports

The challenge is that WebRTC ICE candidates need to advertise an IP address that clients can actually connect to (the host's public IP), while the application binds to the container's network interface.

## Solution: Dual-IP Configuration

This application supports a dual-IP mode specifically for Docker bridge networking:

1. **Bind to container's private interface** - The UDP socket binds to `BIND_IP` (default: `0.0.0.0`)
2. **Advertise public IP in ICE candidates** - WebRTC candidates use `PUBLIC_IP` for client discovery
3. **Port mapping** - Docker forwards UDP traffic from host to container

## Environment Variables

### Required

- **`PUBLIC_IP`**: The public/external IP address that clients will connect to
  - This should be your Docker host's IP address
  - If not set, the application runs in standard mode (auto-detects local IP)

### Optional

- **`BIND_IP`**: The IP address to bind the UDP socket to (default: `0.0.0.0`)
  - `0.0.0.0` allows binding to all interfaces (recommended)
  - You can specify a specific container IP if needed

## Quick Start with Docker Compose (Recommended)

### 1. Configure Environment Variables

Copy the example environment file and configure your PUBLIC_IP:

```bash
cp .env.example .env
```

Edit `.env` and set your public IP:

```bash
# Get your public IP
curl ifconfig.me

# Edit .env file
PUBLIC_IP=<YOUR_PUBLIC_IP>
```

### 2. Start the Service

```bash
docker-compose up -d
```

### 3. View Logs

```bash
docker-compose logs -f str0m
```

### 4. Stop the Service

```bash
docker-compose down
```

## Manual Docker Commands

If you prefer not to use docker-compose, you can run the container directly.

### Building the Docker Image

```bash
docker build -t str0m-docker .
```

### Getting Your Public IP

First, determine your host's public IP address:

```bash
# For cloud VMs (AWS, GCP, Azure, etc.)
curl ifconfig.me

# For local development with known IP
ip addr show
```

### Basic Run Command

```bash
docker run -d \
  --name str0m \
  -p 3000:3000 \
  -p 10000-20000:10000-20000/udp \
  -e PUBLIC_IP=<YOUR_HOST_PUBLIC_IP> \
  str0m-docker
```

Replace `<YOUR_HOST_PUBLIC_IP>` with your actual host IP.

### Example: Running on AWS EC2

```bash
# Get EC2 public IP
PUBLIC_IP=$(curl -s http://169.254.169.254/latest/meta-data/public-ipv4)

# Run container
docker run -d \
  --name str0m \
  -p 3000:3000 \
  -p 10000-20000:10000-20000/udp \
  -e PUBLIC_IP=${PUBLIC_IP} \
  str0m-docker
```

### Example: Development with Known IP

```bash
docker run -d \
  --name str0m \
  -p 3000:3000 \
  -p 10000-20000:10000-20000/udp \
  -e PUBLIC_IP=192.168.1.100 \
  str0m-docker
```

## Port Mapping Explained

### TCP Port 3000 (HTTPS Web Server)
- **`-p 3000:3000`**: Maps host port 3000 to container port 3000
- Used for the HTTPS web server that serves the HTML page and handles WebRTC signaling

### UDP Ports 10000-20000 (WebRTC Media)
- **`-p 10000-20000:10000-20000/udp`**: Maps a range of UDP ports
- WebRTC assigns ephemeral UDP ports for media streams
- The range provides capacity for multiple concurrent connections
- Each connection typically uses one UDP port

**Note**: The UDP port range is generous. For limited connections, you can reduce it, but ensure you map enough ports for your expected concurrent users.

## How It Works

### Network Flow

1. **Client connects to HTTPS server**:
   ```
   Client -> https://<PUBLIC_IP>:3000
   ```

2. **WebRTC negotiation (SDP Offer/Answer)**:
   - Client sends SDP offer via HTTPS POST
   - Server creates RTC instance and binds UDP socket to `BIND_IP:0` (ephemeral port)
   - Server adds ICE candidate with `PUBLIC_IP:<bound-port>`
   - Server returns SDP answer with the public candidate

3. **ICE connectivity check**:
   ```
   Client -> STUN -> <PUBLIC_IP>:<udp-port>
   Docker -> Port forward -> Container:<udp-port>
   ```

4. **Media streaming**:
   ```
   Client <-> UDP <-> <PUBLIC_IP>:<port> <-> Docker Bridge <-> Container
   ```

### Code Implementation

The key logic is in `src/main.rs`:

```rust
let (socket, candidate_addr) = if let Some(config) = docker_config {
    // Docker bridge mode: bind to BIND_IP, advertise PUBLIC_IP
    let socket = UdpSocket::bind(format!("{}:0", config.bind_ip))?;
    let local_addr = socket.local_addr()?;

    // Create candidate with public IP but the port we actually bound to
    let candidate_addr = SocketAddr::new(config.public_ip, local_addr.port());

    (socket, candidate_addr)
} else {
    // Standard mode: auto-detect
    // ...
}
```

## Verifying It's Working

### Check Container Logs

```bash
docker logs str0m
```

You should see:
```
Running in Docker bridge mode: bind=0.0.0.0, public=<YOUR_PUBLIC_IP>
Connect a browser to https://<YOUR_PUBLIC_IP>:3000
```

### Check Listening Ports

Inside the container:
```bash
docker exec str0m netstat -tuln
```

You should see:
- TCP `0.0.0.0:3000` (HTTPS server)
- UDP `0.0.0.0:<random-port>` for each WebRTC connection

## Troubleshooting

### Connection Fails / No Video

**Issue**: Client can't establish WebRTC connection

**Possible causes**:
1. **Incorrect PUBLIC_IP**: Ensure `PUBLIC_IP` is reachable from the client
   ```bash
   # Test from client machine
   ping <PUBLIC_IP>
   ```

2. **Firewall blocking UDP**: Ensure UDP ports 10000-20000 are open
   ```bash
   # On host, check firewall rules
   sudo iptables -L -n | grep 10000

   # AWS Security Group: Allow UDP 10000-20000 inbound
   # GCP Firewall: Allow UDP 10000-20000
   ```

3. **Port mapping incorrect**: Verify Docker port mappings
   ```bash
   docker port str0m
   ```

### Certificate Errors

**Issue**: Browser shows SSL certificate warning

**Cause**: Using self-signed certificates

**Solutions**:
- Accept the certificate warning (for development)
- Use real certificates from Let's Encrypt (for production)
- Replace `cer.pem` and `key.pem` with your own certificates

### UDP Port Exhaustion

**Issue**: New connections fail after several clients

**Cause**: Not enough UDP ports mapped

**Solution**: Increase the UDP port range
```bash
docker run -p 10000-30000:10000-30000/udp ...
```

## Production Considerations

### Security

1. **Use real TLS certificates**: Replace self-signed certificates
2. **Restrict HTTPS access**: Use reverse proxy (nginx, traefik) for TLS termination
3. **Network policies**: Limit which IPs can connect

### Scalability

1. **UDP port planning**: Each connection uses one UDP port
   - 10000-20000 = 10,001 ports = ~10,000 concurrent connections
   - Adjust range based on expected load

2. **Resource limits**: Set Docker resource constraints
   ```bash
   docker run --memory=2g --cpus=2 ...
   ```

### Monitoring

1. **Health checks**: Add Docker health check
2. **Metrics**: Export WebRTC statistics (str0m supports stats via events)
3. **Logging**: Adjust log levels via `RUST_LOG` environment variable
   ```bash
   docker run -e RUST_LOG=str0m=info,str0m_docker=info ...
   ```

## Alternative: Host Networking Mode

If you don't need network isolation, consider using Docker's host networking mode:

```bash
docker run --network=host str0m-docker
```

With host mode:
- No need for `PUBLIC_IP` environment variable
- No port mapping needed
- Application runs in standard mode (auto-detects host IP)
- Simpler configuration but less isolation

## References

- [str0m WebRTC library](https://github.com/algesten/str0m)
- [WebRTC ICE Protocol](https://datatracker.ietf.org/doc/html/rfc8445)
- [Docker Networking](https://docs.docker.com/network/)

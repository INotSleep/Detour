#!/bin/bash
# A throwaway SSH login for Apple's App Store review: a locked-down container on its own port,
# nothing of the host reachable from it. Run as root on the server; again to rebuild it.
#   PORT (default 2222)  the SSH port reviewers connect to
# Prints the login at the end. Remove: docker rm -f detour-demo; the DOCKER-USER rule goes with
# `iptables -D DOCKER-USER -s 172.31.250.0/24 -m conntrack --ctstate NEW -j DROP`.
set -euo pipefail
PORT=${PORT:-2222}
NAME=detour-demo
NET=detour-demo-net
SUBNET=172.31.250.0/24

command -v docker > /dev/null || { echo "Docker is not installed here." >&2; exit 1; }
if ss -ltn "sport = :$PORT" | grep -q LISTEN && ! docker ps --format '{{.Names}}' | grep -qx "$NAME"; then
  echo "Port $PORT is taken: run with PORT=<another one>." >&2
  exit 1
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
# (Not `tr < /dev/urandom | head`: head closing the pipe kills tr, and pipefail the script.)
password=$(openssl rand -base64 18 | tr -dc 'A-Za-z0-9')

cat > "$work/Dockerfile" <<'EOF'
FROM alpine:3.20
RUN apk add --no-cache openssh-server bash htop \
 && adduser -D -s /bin/bash demo \
 && ssh-keygen -A \
 && mkdir -p /home/demo/app/releases /home/demo/logs /home/demo/backups
COPY sshd_config /etc/ssh/sshd_config
COPY home/ /home/demo/
RUN chown -R demo:demo /home/demo
EXPOSE 22
CMD ["/usr/sbin/sshd", "-D", "-e"]
EOF

# Password login only for demo; no forwarding of any kind (the box can't be a proxy).
cat > "$work/sshd_config" <<'EOF'
Port 22
PermitRootLogin no
AllowUsers demo
PasswordAuthentication yes
KbdInteractiveAuthentication no
AllowTcpForwarding no
AllowStreamLocalForwarding no
AllowAgentForwarding no
X11Forwarding no
PermitTunnel no
GatewayPorts no
MaxSessions 4
MaxAuthTries 4
ClientAliveInterval 60
Subsystem sftp internal-sftp
EOF

mkdir -p "$work/home/logs" "$work/home/app" "$work/home/backups"
cat > "$work/home/logs/app.log" <<'EOF'
09:40:02 INFO  deploy started v2.14.0
09:40:05 INFO  GET /health 200 4ms
09:40:07 WARN  slow query 412ms
09:40:09 INFO  POST /api/login 200 38ms
09:40:11 ERROR GET /api/report 500 1.2s
09:40:12 INFO  db 10.0.2.5:5432 ok
09:40:14 INFO  deploy finished
EOF
printf 'services:\n  web:\n    image: nginx:1.27\n    ports: ["80:80"]\n' > "$work/home/app/docker-compose.yml"
printf 'Detour demo server for App Review.\nTry: tail logs/app.log, htop, or the Files button.\n' > "$work/home/README.md"
printf 'export PS1="\\u@detour-demo:\\w\\$ "\n' > "$work/home/.bashrc"

docker build -q -t detour-demo "$work" > /dev/null
docker network inspect "$NET" > /dev/null 2>&1 || docker network create --subnet "$SUBNET" "$NET" > /dev/null
docker rm -f "$NAME" > /dev/null 2>&1 || true
docker run -d --name "$NAME" --restart unless-stopped --network "$NET" -p "$PORT:22" \
  --memory 128m --cpus 0.5 --pids-limit 64 \
  --cap-drop ALL --cap-add CHOWN --cap-add SETUID --cap-add SETGID --cap-add SYS_CHROOT --cap-add AUDIT_WRITE \
  --security-opt no-new-privileges detour-demo > /dev/null
echo "demo:$password" | docker exec -i "$NAME" chpasswd

# Nothing new goes out from the container: not to the host's other services, not to the internet.
iptables -C DOCKER-USER -s "$SUBNET" -m conntrack --ctstate NEW -j DROP 2> /dev/null \
  || iptables -I DOCKER-USER -s "$SUBNET" -m conntrack --ctstate NEW -j DROP

echo "Host: this server ($(hostname -I | cut -d' ' -f1))"
echo "Port: $PORT"
echo "User: demo"
echo "Password: $password"

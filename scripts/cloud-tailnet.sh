#!/bin/bash
[ "${CLAUDE_CODE_REMOTE:-}" = true ] && [ -n "${OP_SERVICE_ACCOUNT_TOKEN:-}" ] || exit 0
command -v ssh >/dev/null || apt-get install -y -qq openssh-client >/dev/null 2>&1
sock=/tmp/tailscaled.sock
tailscaled --tun=userspace-networking --socket=$sock --state=mem: >/tmp/tailscaled.log 2>&1 &
sleep 3
tailscale --socket=$sock up --authkey="$(op read "op://Agent - Dev/EZH cloud session/TS_AUTHKEY")" --hostname=ezh-cloud
umask 077
mkdir -p ~/.ssh
op read "op://Agent - Dev/EZH cloud session/private_key" >~/.ssh/ezh_cloud
for h in zephyrus galleria; do
cat >>~/.ssh/config <<EOF
Host ezh-$h
  HostName $h-01-win
  User hrs05
  IdentityFile ~/.ssh/ezh_cloud
  IdentitiesOnly yes
  StrictHostKeyChecking accept-new
  ProxyCommand tailscale --socket=$sock nc %h %p
EOF
done

#!/bin/bash
[ "${CLAUDE_CODE_REMOTE:-}" = true ] && [ -n "${TS_AUTHKEY:-}" ] || exit 0
sock=/tmp/tailscaled.sock
tailscaled --tun=userspace-networking --socket=$sock --state=mem: >/tmp/tailscaled.log 2>&1 &
sleep 3
tailscale --socket=$sock up --authkey="$TS_AUTHKEY" --hostname=ezh-cloud
umask 077
mkdir -p ~/.ssh
echo "$CCW_SSH_KEY_B64" | base64 -d >~/.ssh/ezh_cloud
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

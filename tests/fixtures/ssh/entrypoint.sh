#!/bin/sh
set -eu

ssh_user="${SSH_USER:-lazydb}"
ssh_uid="${SSH_UID:-1000}"
ssh_gid="${SSH_GID:-1000}"

if ! grep -q "^${ssh_user}:" /etc/group; then
    addgroup -g "$ssh_gid" "$ssh_user"
fi
if ! id "$ssh_user" >/dev/null 2>&1; then
    adduser -D -H -u "$ssh_uid" -G "$ssh_user" -s /bin/sh "$ssh_user"
fi
passwd -d "$ssh_user" >/dev/null

install -d -m 0700 -o "$ssh_user" -g "$ssh_user" "/home/${ssh_user}"
install -d -m 0700 -o "$ssh_user" -g "$ssh_user" "/home/${ssh_user}/.ssh"
install -m 0600 -o "$ssh_user" -g "$ssh_user" \
    /run/lazydb-ssh-src/authorized_keys "/home/${ssh_user}/.ssh/authorized_keys"

install -d -m 0700 /run/lazydb-ssh
install -m 0600 /run/lazydb-ssh-src/ssh_host_ed25519_key /run/lazydb-ssh/ssh_host_ed25519_key

exec /usr/sbin/sshd -D -e -f /etc/ssh/lazydb/sshd_config

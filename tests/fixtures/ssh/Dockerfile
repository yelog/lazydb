FROM alpine:3.22@sha256:49340b346798875b5592e5d420241f2f9cfba5f47e62ebb1446a1b34734e898f

RUN apk add --no-cache openssh-server=10.0_p1-r11

COPY entrypoint.sh /usr/local/bin/lazydb-sshd-entrypoint
RUN chmod 0755 /usr/local/bin/lazydb-sshd-entrypoint

ENTRYPOINT ["/usr/local/bin/lazydb-sshd-entrypoint"]

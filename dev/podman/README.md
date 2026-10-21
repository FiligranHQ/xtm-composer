# XTM Composer on Podman

The `podman` daemon selector deploys connectors on [Podman](https://podman.io), the standard container
engine of RHEL and SUSE Linux Enterprise, in **rootful** or **rootless** mode. The Composer talks to the
Podman API socket: no Docker daemon is involved.

It is validated in CI against Podman 4.9, rootful and rootless, including pulls from an authenticated
registry.

## Configuration

```yaml
opencti:
  daemon:
    selector: podman
    podman:                                # Optional
      socket_path: /run/podman/podman.sock # Optional, auto-detected by default
      network_mode: opencti_default        # Optional, same container options as the docker block
```

Or with environment variables:

```
OPENCTI__DAEMON__SELECTOR=podman
OPENCTI__DAEMON__PODMAN__SOCKET_PATH=/run/podman/podman.sock
OPENCTI__DAEMON__PODMAN__NETWORK_MODE=opencti_default
```

The same applies to OpenAEV with the `openaev.daemon` block (`OPENAEV__DAEMON__...`).

The `podman` block accepts every option of the `docker` block (`network_mode`, `extra_hosts`, `dns`,
`cap_add`, `cap_drop`, `security_opt`, `ulimits`, ...).

### Socket detection

Without `socket_path`, the Composer uses the first socket that exists:

1. `$XDG_RUNTIME_DIR/podman/podman.sock`: the rootless socket of the user running the Composer
2. `/run/user/<uid>/podman/podman.sock`: the same, when `XDG_RUNTIME_DIR` is not set
3. `/run/podman/podman.sock`: the rootful socket

At startup it logs `Connected to Podman` with the socket, how it was found, the mode detected from the
Podman API (`rootful` or `rootless`), the Podman version and the API version. When no socket exists yet,
it logs the paths it checked and retries until one shows up.

## Rootful Podman

```bash
sudo systemctl enable --now podman.socket
# Socket: /run/podman/podman.sock
```

Only root can use this socket: run the Composer as root, or in a container (see below).

## Rootless Podman

As the user that owns the connectors:

```bash
systemctl --user enable --now podman.socket
# Socket: $XDG_RUNTIME_DIR/podman/podman.sock, usually /run/user/<uid>/podman/podman.sock

# Keep the socket and the connectors running after the user logs out, and start them at boot
sudo loginctl enable-linger <user>
```

## Running the Composer in a container

Mount the Podman socket at `/run/podman/podman.sock` in the Composer container: the Composer finds it
there without configuration. [`compose.yml`](compose.yml) is a complete example, to run with
`podman compose up -d` (or `podman-compose up -d`):

```yaml
volumes:
  - /run/podman/podman.sock:/run/podman/podman.sock                # Rootful
  # - /run/user/1000/podman/podman.sock:/run/podman/podman.sock    # Rootless, replace 1000 with `id -u`
security_opt:
  - label=disable # On SELinux hosts, lets the container use the socket
```

For a rootless Podman, run the Composer container with the same user: its root user is then mapped to
that user, who owns the socket.

## Private registries

### Credentials in the Composer

```
OPENCTI__DAEMON__REGISTRY__SERVER=registry.example.com
OPENCTI__DAEMON__REGISTRY__USERNAME=<username>
OPENCTI__DAEMON__REGISTRY__PASSWORD=<password>
```

Connector images are pulled from `<server>/<image>`. Passwords may contain any character.

### Credentials delegated to Podman

Podman can authenticate with credentials stored on its host instead. Keep only the server in the
Composer configuration (`OPENCTI__DAEMON__REGISTRY__SERVER`), without username and password, and log in
with the auth file the Podman service reads by default and that survives reboots:

- Rootful:

  ```bash
  sudo mkdir -p /root/.config/containers
  sudo podman login --authfile /root/.config/containers/auth.json registry.example.com
  sudo chmod 600 /root/.config/containers/auth.json
  ```

- Rootless, as the user running the connectors:

  ```bash
  podman login --authfile ~/.config/containers/auth.json registry.example.com
  chmod 600 ~/.config/containers/auth.json
  ```

The Podman service reads the credentials at each pull: no restart is needed. A plain `podman login` stores
them in `/run/containers/0/auth.json` (rootful) or `$XDG_RUNTIME_DIR/containers/auth.json` (rootless),
which are cleared at reboot. `REGISTRY_AUTH_FILE` is only read by the `podman` command line: setting it
on the Podman service has no effect on the pulls it serves.

A registry served over plain HTTP, or with a certificate signed by a private authority, must be declared
to Podman: see `insecure` in
[containers-registries.conf(5)](https://github.com/containers/image/blob/main/docs/containers-registries.conf.5.md)
and the CA directory `/etc/containers/certs.d/<registry>/` in
[containers-certs.d(5)](https://github.com/containers/image/blob/main/docs/containers-certs.d.5.md).

## Proxy CA certificates

With `https_proxy_ca`, the Composer writes the CA bundle to `/tmp/xtm-composer-proxy-ca` and mounts it
into the connectors. Podman relabels it for SELinux. When the Composer runs in a container, share that
directory with the host at the same path:

```yaml
volumes:
  - /tmp/xtm-composer-proxy-ca:/tmp/xtm-composer-proxy-ca
```

## Migrating from the `docker` selector

If the Podman socket was mounted at `/var/run/docker.sock` with `selector: docker`:

1. Set `OPENCTI__DAEMON__SELECTOR=podman`.
2. Mount the socket at `/run/podman/podman.sock` instead, or set
   `OPENCTI__DAEMON__PODMAN__SOCKET_PATH=/var/run/docker.sock`.
3. Rename the `OPENCTI__DAEMON__DOCKER__*` options to `OPENCTI__DAEMON__PODMAN__*`.
4. Registry credentials can move back into the Composer configuration, if preferred.

The connectors already deployed are kept.

## Troubleshooting

| Log | Cause |
|---|---|
| `No Podman API socket found` | The socket is not enabled, not mounted in the Composer container, or the rootless user is not lingering. |
| `Could not query the Podman version` | The Composer user cannot use the socket: permissions, or SELinux (`label=disable`). |
| `The socket is not served by Podman` | The socket belongs to a Docker daemon: use the `docker` selector. |
| `Error fetching container image` | The registry rejected the pull: check the server, the credentials, or the auth file used by Podman. |

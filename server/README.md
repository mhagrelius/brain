# brain-server on a Synology

The vault and its vectors, shared between one person's machines. This is what
to do once, in order. It assumes DSM 7.2 or later with Container Manager, an
x86_64 model, and a registry the NAS can already pull from.

## 1. The folders

`/volume1/docker/brain-server/` holds the compose file; `data/` inside it is
the bind mount. Make both in File Station — Create → Create folder — before
anything else, because **Synology's Docker refuses to create a missing
bind-mount source directory** where vanilla Docker would make it. The error is
`Bind mount failed: … does not exist` and the project will not start.

No `chown` is needed, and attempting one is a trap: see the `user: "0:0"` note
in `docker-compose.yml`. On a Synology the container runs as root because
`/volume1`'s ACLs override POSIX ownership, so a chown to a non-root uid
reports success and the writes still fail.

## 2. Push the image

On the machine with the source:

```sh
BRAIN_REGISTRY=your-nas:5000 ./packaging/deploy-server.sh
```

It runs `./test.sh`, builds, smoke-tests the image — serves `/health`, refuses
an unauthenticated write — and only then pushes. It prints the tag to use. Add
`BRAIN_TEST_ARGS=--headless` if you are on SSH with no display.

**If your registry is plain HTTP**, which a self-hosted one usually is, the
push is refused and the script tells you so rather than quietly downgrading:

```sh
BRAIN_REGISTRY_INSECURE=1 BRAIN_REGISTRY=your-nas:5050 ./packaging/deploy-server.sh
```

### Name the image `localhost` when the NAS pulls it

The pulling machine has to accept HTTP as well, and this is where it is easy to
do far more work than necessary.

**A registry stores repositories by name, not by hostname.** An image pushed as
`your-nas:5050/brain-server:tag` is the same repository as
`localhost:5050/brain-server:tag`. And Docker treats a registry reached over
localhost as insecure *by default*. So when the registry and the container both
run on the NAS, writing

```
BRAIN_SERVER_IMAGE=localhost:5050/brain-server:2026-08-04
```

in `.env` pulls over HTTP with no configuration anywhere.

Referring to it by the NAS's own network name instead means an
`insecure-registries` entry in the daemon config — on Synology that is
`/var/packages/ContainerManager/etc/dockerd.json`, editable only over SSH, and
it needs the package restarted. There is no reason to do that here.

Adding the registry under Container Manager → **Registry** → **Settings** does
not help either way: that dialog offers only "Trust SSL Self-Signed
Certificate" with no plain-HTTP option, and the list it maintains feeds the
GUI's browse-and-download tab rather than what a Project can pull.

## 3. The project

Container Manager → **Project** → **Create**, name `brain-server`, path
`/volume1/docker/brain-server`, source **Upload docker-compose.yml**, and give
it `server/docker-compose.yml`.

**Upload it; do not paste it.** Container Manager's compose editor auto-indents
— it carries the previous line's indentation forward and adds yours to it — so
typed YAML is mangled within a few lines. The `YAML Configurations` tab on an
existing project is read-only, including when stopped, so changes go the same
way: overwrite `compose.yaml` in the project folder via File Station and Build
again. Container Manager stores it as `compose.yaml` whatever you called it.

Put `.env` beside it with the token — `openssl rand -hex 32`; the server
refuses to start on anything under 32 characters — or inline the values in the
compose file if that is easier than getting a dotfile onto the NAS.

The compose file and the token sit at the project root, outside `data/`, so
they are never visible from inside the container.

## 4. Reaching it

`BRAIN_BIND` in `.env` decides who can. **This speaks plain HTTP** — there is
no TLS and there is not going to be, because the thing it was built to sit
behind already encrypts.

- Unset: every interface, so every device on the LAN. The token then crosses
  that LAN in clear text.
- Set to the NAS's Tailscale address (`tailscale ip -4`): only over the
  tailnet, where WireGuard is doing the work. This is the arrangement to want.

Either way, **do not forward 8082 on the router.** `/health` is the only
unauthenticated route and it says nothing but a count, but the rest of it is
your notes behind one shared secret and no rate limiting.

## 5. Point Brain at it

In `~/.config/brain/config.json` on each machine:

```json
{
  "sync_url": "http://100.x.y.z:8082",
  "sync_token": "the same token",
  "vectors_url": "http://100.x.y.z:8082",
  "vectors_token": "the same token"
}
```

Both halves are the same service and the same secret; they are separate keys
because they are separately optional. Sync is off until both `sync_url` and
`sync_token` are set — half a configuration is treated as none, so a typo in
one of them turns syncing off rather than half on.

A pass runs every 60 seconds. Local edits are instant regardless; the delay is
only in noticing another machine.

## Checking it works

From a machine on the tailnet:

```sh
curl http://100.x.y.z:8082/health
# {"ok":true,"vectors":0}
```

Then the real thing, which drives the actual client against it:

```sh
cargo run --release --example sync_check -- http://100.x.y.z:8082 "$TOKEN"
```

It writes one throwaway note, pushes it, pulls it to a second temporary vault,
makes them disagree, checks the conflict copy lands beside the original, and
deletes the note again. It does not touch your vault.

`./sync-check.sh` does the same against a server it starts and stops itself,
which is the one to run when the question is "is the code right" rather than
"is the NAS right".

## Backups, and what is actually at risk

`/volume1/docker/brain-server/vault/` is ordinary Markdown. Back it up the way
you back up anything else on the NAS, and consider `git init` in it — the
server only ever writes whole files, so the history is clean.

`vectors.json` beside it is a cache. Losing it costs one pass of embedding.

The thing worth being careful with is the **base snapshot**, which is not here
— it is `.brain/sync.json` inside each *client's* vault. It records what that
machine and the server last agreed on. Deleting it is safe but slow: the next
pass treats everything as new, which pushes and pulls the whole vault and calls
nothing a conflict that is not a genuine one.

## Updating

Re-run `deploy-server.sh`, put the new tag in `.env`, and rebuild the project
in Container Manager. Pin the dated tag rather than `:latest` — a restart
should not be able to quietly change what is holding your notes.

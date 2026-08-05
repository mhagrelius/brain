# brain-server on a Synology

The vault and its vectors, shared between one person's machines. This is what
to do once, in order. It assumes DSM 7.2 or later with Container Manager, an
x86_64 model, and a registry the NAS can already pull from.

## 1. The folder, and the one command that is not optional

Over SSH on the NAS:

```sh
sudo mkdir -p /volume1/docker/brain-server
sudo chown -R 10001:10001 /volume1/docker/brain-server
```

**The uid has to match.** The image runs as 10001 and the kernel checks that
against the bind mount's owner. Skip this and you get the worst kind of
failure: the container starts, `/health` answers, Container Manager shows it
green, and every write fails. If notes are not appearing, check this first.

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
BRAIN_REGISTRY_INSECURE=1 BRAIN_REGISTRY=your-nas:5000 ./packaging/deploy-server.sh
```

The NAS has to agree as well: Container Manager → **Registry** → **Settings**,
add the address, and tick the insecure-connection option. That is a reasonable
trade when the registry only answers on the tailnet. It is not one if it
answers anywhere else — put a certificate on it instead.

## 3. The project

Copy `server/docker-compose.yml` and `server/.env.example` into
`/volume1/docker/brain-server-project/` on the NAS, rename the second to
`.env`, and fill it in. Generate the token with `openssl rand -hex 32`; the
server refuses to start on anything under 32 characters.

Then Container Manager → **Project** → **Create**, path
`/volume1/docker/brain-server-project`, and let it read the compose file.

Note the data folder and the project folder are different: the compose file and
the token live in the second, the notes in the first. Putting the token inside
the folder that gets synced would be its own kind of mistake.

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

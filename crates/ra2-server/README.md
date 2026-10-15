# ra2-server

The meeting-place server for Internet games: it keeps the list of games waiting
for players and passes messages between the players of each game, so nobody
needs a reachable address of their own.

It contains no game code and no game files. It depends only on `ra2-net`
(std-only Rust); what the players send each other is opaque bytes to it.

## Run

```
cargo run --release -p ra2-server            # port 47100
cargo run --release -p ra2-server -- 5000    # another port
PORT=5000 ra2-server
```

It listens on every address, over plain TCP, and logs connection and game
counts to standard output.

## Container

From the repository root (only the two networking crates are copied in):

```
docker build -f crates/ra2-server/Dockerfile -t ra2-server .
docker run -p 47100:47100 ra2-server
```

For ECS or similar: one task, one TCP port (47100), a network load balancer or
a public IP in front. State is in memory only: restarting the task drops the
list and any games in progress, and it must run as a single instance (two
tasks would each have their own list).

## Players

In the game: main menu, Internet. Type the server's address (`host:port`), or
start the game with `RA2_SERVER=host:port`.

## What it does not do

- No encryption and no accounts. Passwords are sent and compared in the clear;
  they keep strangers out of a game, nothing more.
- No limits on connections or message rates beyond a 1 MB cap per message.
- Games in progress are relayed through the game's creator: if the creator's
  connection goes, the room closes.

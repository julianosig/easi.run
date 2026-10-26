# Install easi.run

These are instructions for a coding agent (Claude Code, Codex, or another) that a person asked to install easi.run on
the Linux server it runs on. easi.run runs real Chrome sessions for agents, which the person can watch and take over.

## Steps

1. Check who you run as: `id -u`. If it prints `0` (root), stop: ask the person to create an account in the docker
   group (`adduser NAME && usermod -aG docker NAME`), and to run you as that account. The installer needs no sudo: the
   docker group is all it needs, and it makes its few changes as root through a short-lived privileged Docker
   container.

2. Check all the software easi.run needs at once, before anything else, so that nothing is found missing one at a
   time:
   - `git`, `curl`, `docker`, `ss`, `systemctl`, `awk`, `stat`, `df`, `timeout` and `flock` (`command -v`);
   - `iptables`, also in `/usr/sbin` and `/sbin`, which the account's `PATH` may leave out;
   - Docker Engine 28 or later, run by `docker.service` (`docker version`, `systemctl is-active docker`), with the
     Docker Compose v2 and Buildx plugins (`docker compose version`, `docker buildx version`), and the account in the
     docker group.

   If all of it is there, go on to step 3. If anything is missing or too old, do not give the person a list of
   commands. Write one script that sets it all up:
   - in a private directory, made with `mktemp -d /tmp/easi-run-setup.XXXXXX`, as `setup.sh`;
   - installing only what is missing, with this distribution's package manager. Docker Engine and its Compose plugin
     come from Docker's own repository (https://docs.docker.com/engine/install), not the snap or Docker Desktop; the
     script enables and starts it (`systemctl enable --now docker`) and adds the person's account to the docker group;
   - starting with `#!/bin/sh` and `set -eu`, and short enough to read.

   Show it to the person and ask them to run it once as root, as an administrator of this server: `sudo sh` and its
   path, or, where this account cannot use sudo, `su -c 'sh PATH'`. The docker group takes effect at the account's
   next login: ask them to log in again and to start you again, then check once more.

3. Download the installer, then read all of it before you run anything:

   ```sh
   curl -fsSLO https://easi.run/install.sh
   ```

   Check the server without changing anything:

   ```sh
   bash install.sh --dry-run
   ```

   If it says a tool is missing, set it up as in step 2. If any other check fails (memory, disk, the port, the
   operating system), tell the person what it says (it names what is wrong and how to fix it), and stop.

4. Install, as the current user. Nothing is compiled: it downloads about 550 MB (the images, and Google Chrome), which
   takes one to two minutes with a fast connection and longer with a slow one. Run it in the foreground with a timeout
   of at least 10 minutes. If your command timeout is shorter, run it in the background, with its output in a file:

   ```sh
   nohup sh -c 'bash install.sh; echo "install.sh exited with status $?"' > install.log 2>&1 &
   ```

   Then read `tail -n 20 install.log` every 30 seconds or so, until its last line is
   `install.sh exited with status N`. Status 0 means it finished: the summary is just above. Any other status means it
   stopped, and the lines above say why. If it is interrupted, run it again: it reuses what it already downloaded.

5. Tell the person what the summary says:
   - the admin's address, `https://localhost:9443`, and, for another computer, the summary's `Tunnel:` line, exactly
     as it printed it. Never make up or change its user or address: where it shows `you@your-server`, tell them to use
     the command they already use to log in to this server, adding the line's `-N -L` part as it printed it. Its
     `Editors:` line says what VS Code and Cursor over Remote-SSH do instead: give that too;
   - the summary's `Password:` line: unless it says the password is set already, they create it themselves at the
     admin's address, right after the install (from another computer, through the tunnel of the `Tunnel:` line);
   - whether MCP is ready, as the summary says: ready means the server `easi` is in Claude Code, available from its
     next session; otherwise the summary says why, or prints the line that adds it. It also prints the line for Codex
     and for an agent on another computer.

## Never

- Run the installer as root (`sudo bash install.sh`, say): it refuses, and needs no sudo.
- Ask for the admin's password, or choose or set one for the person: they create it themselves, on the admin's first
  screen.
- Open the admin yourself, or call its API: its first screen is the person's.
- Guess the person's SSH user or the server's address for them: give the summary's lines as it printed them.
- Edit `install.sh`, skip its firewall step, or start the stack another way to get past a failed check.

## After

Tell the person to start a new Claude Code session (or to add the Codex line the summary printed). There, ask
easi.run to open a session: `start_session` with a URL returns the session and its `watch_url`, where the person
watches it live and presses **Take control** whenever they want. The documentation is at https://docs.easi.run.

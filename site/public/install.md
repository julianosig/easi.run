# Install easi.run

These are instructions for a coding agent (Claude Code, Codex, or another) that a person asked to install easi.run on
the Linux server it runs on. easi.run runs real Chrome sessions for agents, which the person can watch and take over.

## Steps

1. Download the installer, then read all of it before you run anything:

   ```sh
   curl -fsSLO https://easi.run/install.sh
   ```

2. Check the server without changing anything:

   ```sh
   bash install.sh --dry-run
   ```

   If a check fails, tell the person what it says (it names what is missing and how to add it), and stop.

3. Install, as the current user:

   ```sh
   bash install.sh
   ```

   It asks sudo once, for the firewall rule only. If it stops because sudo needs a password and there is no terminal
   to ask it in, ask the person to run `bash install.sh` themselves in a terminal on this server, then continue from
   step 4 with what it printed.

4. Tell the person what the summary says:
   - the admin's address, `https://localhost:9443`, and, from another computer, the `ssh -N -L 9443:…` line that
     reaches it;
   - where the admin's password is, as the summary says: for a new one, the file `~/.config/easi-run/password`, which
     they read in their own terminal;
   - whether MCP is ready, as the summary says: ready means the server `easi` is in Claude Code, available from its
     next session; otherwise the summary says why, or prints the line that adds it. It also prints the line for Codex
     and for an agent on another computer.

## Never

- Run the installer as root, or with `sudo bash install.sh`: it refuses, and needs sudo for the firewall rule only.
- Print, copy or repeat the admin's password in the chat: give the person the file's path.
- Edit `install.sh`, skip its firewall step, or start the stack another way to get past a failed check.

## After

Ask easi.run to open a session: `start_session` with a URL returns the session and its `watch_url`, where the person
watches it live and presses **Take control** whenever they want. The documentation is at https://docs.easi.run.

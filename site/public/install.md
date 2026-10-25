# Install easi.run

These are instructions for a coding agent (Claude Code, Codex, or another) that a person asked to install easi.run on
the Linux server it runs on. easi.run runs real Chrome sessions for agents, which the person can watch and take over.

## Steps

1. Check who you run as: `id -u`. If it prints `0` (root), stop: ask the person to create an account with sudo, in
   the docker group (`adduser NAME && usermod -aG sudo,docker NAME`), and to run you as that account.

2. Download the installer, then read all of it before you run anything:

   ```sh
   curl -fsSLO https://easi.run/install.sh
   ```

3. Check the server without changing anything:

   ```sh
   bash install.sh --dry-run
   ```

   If a check fails, tell the person what it says (it names what is missing and how to add it), and stop.

4. Install, as the current user. The first run takes about 10 to 15 minutes on 2 cores, most of it building the
   images: longer than an agent's command timeout usually allows. Run it in the background, with its output in a
   file:

   ```sh
   nohup sh -c 'bash install.sh; echo "install.sh exited with status $?"' > install.log 2>&1 &
   ```

   Then read `tail -n 20 install.log` every minute or so, until its last line is `install.sh exited with status N`.
   Status 0 means it finished: the summary is just above. Any other status means it stopped, and the lines above say
   why. Or run it in the foreground with a timeout of at least 30 minutes. If it is interrupted, run it again: it
   reuses the steps it already built.

   It asks sudo once, for the firewall rule only. If it stops because sudo needs a password and there is no terminal
   to ask it in, ask the person to run `bash install.sh` themselves in a terminal on this server, then continue from
   step 5 with what it printed.

5. Tell the person what the summary says:
   - the admin's address, `https://localhost:9443`, and, from another computer, the `ssh -N -L 9443:…` line that
     reaches it;
   - the setup link on the summary's `Password:` line (`https://localhost:9443/#setup=…`), where they create the
     admin's password themselves: give them the whole link, and say it works once, for up to 15 minutes, and that from
     another computer they open the `ssh -N -L 9443:…` tunnel first. If the summary says the password is set already,
     say so; if it says how to get the link, run that command and give them the link it prints;
   - whether MCP is ready, as the summary says: ready means the server `easi` is in Claude Code, available from its
     next session; otherwise the summary says why, or prints the line that adds it. It also prints the line for Codex
     and for an agent on another computer.

## Never

- Run the installer with `sudo bash install.sh`: it refuses, and needs sudo for the firewall rule only.
- Ask for the admin's password, or choose one for the person: they create it themselves, on the setup link's
  screen.
- Open or use the setup link yourself: it is the person's, once.
- Edit `install.sh`, skip its firewall step, or start the stack another way to get past a failed check.

## After

Tell the person to start a new Claude Code session (or to add the Codex line the summary printed). There, ask
easi.run to open a session: `start_session` with a URL returns the session and its `watch_url`, where the person
watches it live and presses **Take control** whenever they want. The documentation is at https://docs.easi.run.

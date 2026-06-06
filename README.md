<img src="site/purple-logo.svg" alt="purple" width="213" height="48">

**An open-source terminal SSH manager for macOS and Linux that keeps `~/.ssh/config` in sync with your cloud infra.** 

Spin up a VM on AWS, GCP, Azure, Hetzner, Proxmox or 11 other cloud providers and it shows up in your host list. Destroy it and the entry dims. No more hand-editing `~/.ssh/config` after every Terraform run, no more grepping cloud consoles for the right IP.

A fast Rust TUI with fuzzy search across hundreds of hosts, file transfer, Docker and Podman over SSH, multi-host SSH key push, short-lived HashiCorp Vault SSH certificates and an MCP server for AI agents.

Keyboard-driven. Single binary. MIT licensed.

[![crates.io](https://img.shields.io/crates/v/purple-ssh?color=b44aff&labelColor=0a0a14)](https://crates.io/crates/purple-ssh)
[![downloads](https://img.shields.io/crates/d/purple-ssh?color=b44aff&labelColor=0a0a14)](https://crates.io/crates/purple-ssh)
[![mit](https://img.shields.io/badge/license-mit-b44aff?labelColor=0a0a14)](LICENSE)
[![built with ratatui](https://img.shields.io/badge/built_with-ratatui-b44aff?labelColor=0a0a14&logo=ratatui&logoColor=fff)](https://ratatui.rs/)
[![Website](https://img.shields.io/badge/website-getpurple.sh-00f0ff?labelColor=0a0a14)](https://getpurple.sh)

![purple terminal SSH client demo](demo.gif)

## Install

```
curl -fsSL getpurple.sh | sh
```

<details>
<summary>brew, cargo, nix, AUR or from source</summary>

```
brew install erickochen/purple/purple
```
```
cargo install purple-ssh
```
```
nix profile install github:erickochen/purple
```
```
paru -S purple-bin
```
```
yay -S purple-bin
```
```
git clone https://github.com/erickochen/purple.git
cd purple && cargo build --release
```
</details>

Claude Desktop users can install the [.mcpb bundle](https://github.com/erickochen/purple/releases/latest) for one-click MCP integration (read-only by default). Setup details on the [MCP Server wiki](https://github.com/erickochen/purple/wiki/MCP-Server). No data leaves your machine. See [PRIVACY.md](PRIVACY.md).

Run `purple`. Press `?` on any screen for help. That's it.

## Why I built this

My SSH config was fine. Proper aliases, ProxyJump chains, organized by provider. Not the problem.

The problem was everything around it. Need to check a container? `ssh host docker ps`. Copy a file? `scp` with the right flags. Run the same command on ten hosts? Write a loop or boot up Ansible for a one-liner. Spin up a VM on Hetzner? Open the console, grab the IP, edit config, save. Someone asks which box runs what? Good luck.

I wanted one place for all of that. So I built it.

## What you get

<img src="screenshots/detail.png" width="70%" align="left" alt="detail panel">

🔍 **Everything at a glance.** Connection info, jump route, activity sparkline, tags, tunnels, snippets, containers and server metadata. Health dots show which hosts are up. Group by provider, tag or flat.

<br clear="both">
<br>

🔎 **Jump to anything with one keystroke.** Press `:` for a universal fuzzy bar across hosts, tunnels, containers, snippets and actions. Searches the SSH `User`, `ProxyJump` and Vault SSH role too, so typing your username finds every server you log in as. Field prefixes (`user:`, `proxy:`, `vault:`, `tag:`) scope to a single directive. Like Linear's `Cmd+K`, but in your terminal.

![jump bar](screenshots/jump.png)

⚡ **Instant fuzzy search.** Names, IPs, tags, users. Frecency sorting puts your most-used hosts on top. Works the same with 5 hosts or 500. Scoped search within groups.

![fuzzy search](screenshots/search.png)

☁️ **Your ssh config tracks your infra.** Drop in one API token per provider. New VMs land in `~/.ssh/config` the moment they boot. IPs stay current as instances move. Decommissioned hosts dim so you can purge them on your terms. 16 providers including AWS, GCP, Azure, Hetzner, DigitalOcean and Proxmox. Run multiple accounts per provider side by side. See the [wiki](https://github.com/erickochen/purple/wiki/Cloud-Providers) for the full list.

![cloud providers](screenshots/providers.png)

🐳 **Manage every container in your fleet from one tab.** Every Docker and Podman container grouped per host. Shell in, stream logs, restart, stop, exec or kick a whole compose stack member by member. No remote agent, no extra ports. Just SSH.

![containers](screenshots/containers.png)

🚇 **Live tunnel monitoring.** Every SSH forward with throughput, channels, clients and uptime. Local, Remote and Dynamic SOCKS. The detail panel reveals which app owns each open channel, in real time.

![live tunnels](screenshots/tunnels.png)

🗝️ **Push any SSH key to your fleet from one tab.** Dedicated Keys page with strength score, randomart fingerprint and a per-key activity sparkline. Press `p` to push the highlighted key to multiple hosts at once. Vault-managed hosts skip automatically so cert-managed hosts stay cert-managed. `Tab` from the Hosts page.

![keys](screenshots/keys.png)

🪄 **Run a command across your fleet from one tab.** Save reusable snippets with parameters, pick any set of hosts and run. purple shows a command's blast radius before you fan it out and tracks every run, so you know how reliable a snippet is before you trust it. `Tab` from the Hosts page.

![multi-host snippets](screenshots/snippets.png)

**And more.**

🚚 Visual file transfer with split-pane explorer.

🗝️ Automatic password retrieval from OS Keychain, 1Password, Bitwarden, pass, the HashiCorp Vault KV secrets engine and Proton Pass.

🎫 Short-lived SSH certificates signed via the HashiCorp Vault SSH secrets engine.

🤖 MCP server for AI agents like Claude Code and Cursor.

See the [wiki](https://github.com/erickochen/purple/wiki) for details.

## How it works

purple reads `~/.ssh/config` directly. No database, no daemon, no account. Comments, indentation, include files, unknown directives. All preserved.

Written in Rust. Single binary. 7300+ tests. MIT license.

## Links

📖 [Wiki](https://github.com/erickochen/purple/wiki) · ☁️ [Cloud Providers](https://github.com/erickochen/purple/wiki/Cloud-Providers) · 🤖 [MCP Server](https://github.com/erickochen/purple/wiki/MCP-Server) · ❓ [FAQ](https://github.com/erickochen/purple/wiki/FAQ) · 🩺 [Troubleshooting](https://github.com/erickochen/purple/wiki/Troubleshooting) · 🔒 [Security](SECURITY.md) · 🧠 [llms.txt](https://getpurple.sh/llms.txt)

## Credits

Screenshots and demo videos are captured in [Ghostty](https://ghostty.org) with [Berkeley Mono™](https://usgraphics.com/products/berkeley-mono) by U.S. Graphics Company.

<details>
<summary>Ghostty config</summary>

```
theme = Dracula

font-family = Berkeley Mono
font-size = 15
font-feature = +ss01
font-feature = +ss05
font-feature = +calt

window-colorspace = display-p3
window-padding-x = 6
window-padding-y = 6,6
```
</details>

## Feedback

Bug or feature request? [Open an issue](https://github.com/erickochen/purple/issues).

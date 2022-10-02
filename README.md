# RustCON
RustCON is an [RCON](https://developer.valvesoftware.com/wiki/Source_RCON_Protocol) client written in Rust for game server administration.

## Installation

## Usage

## Feature goals
- [x] Authentication
- [x] Send/receive commands
  - [x] Basic RCON command prompt
  - [ ] Correctly receive large (4096+ B) packets that are split
  - [ ] Maintain command history
- [ ] Configs
  - [x] Optionally read password from RCON_PASS env variable
  - [ ] Provide autocomplete
- [ ] Robust Error Handling
  - [x] Re-ask for password when getting invalid auth response
  - [x] Handle invalid password and command inputs
  - [x] Automatcially retry lost connection
  - [ ] Signal handling (Ctrl+c) etc.
- [ ] TUI
  - [x] CLI
  - [ ] Help menu
    - [ ] Description / more info popup
    - [ ] Autofill
  - [ ] Autocomplete
    - [ ] Commands
    - [ ] Subcommands
    - [ ] Users

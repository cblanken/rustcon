# RustCON
RustCON is an [RCON](https://developer.valvesoftware.com/wiki/Source_RCON_Protocol) client written in Rust for game server administration.

## Installation

## Usage

## Feature goals
- [x] Authentication
- [x] Send/receive commands
  - [x] Basic RCON command prompt
  - [ ] Correctly receive large (4096+ B) packets that are split
- [ ] Configs
  - [ ] Optionally read password from RCON_PASS env variable
  - [ ] Optionally provide command info file for autocomplete
- [ ] Robust Error Handling
  - [x] Re-ask for password when getting invalid auth response
  - [ ] Signal handling (Ctrl+c) etc.
  - [ ] Automatcially retry lost connection
  - [ ] Handle invalid password and command inputs
- [ ] TUI
  - [ ] CLI
  - [ ] Help menu
    - [ ] Description / more info popup
    - [ ] Autofill
  - [ ] Autocomplete
    - [ ] Commands
    - [ ] Subcommands
    - [ ] Users

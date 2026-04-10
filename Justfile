set shell := ["sh", "-cu"]

default:
    @just --list

build:
    cargo build --release

install: build
    ./scripts/install-local.sh

restart-panel:
    pkill -x cosmic-panel

install-restart: install
    pkill -x cosmic-panel

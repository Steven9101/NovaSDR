#!/bin/bash
set -e

# Make sure we are inside of the source directory
cd $( cd -- "$( dirname -- "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )

# Pull and get frontend submodule
git pull
git submodule update --init --recursive

# Build the server
cargo build -p novasdr-server --release --features "soapysdr,clfft"

# Build the frontend
cd frontend
npm ci
npm run build

cd ..

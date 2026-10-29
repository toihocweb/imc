default: update

update:
    git pull --ff-only
    cd web-dashboard && npm run build
    cargo build --release -j 2 -p imc-cli
    ./target/release/imc install
    rm -rf target/release/incremental

clean-update:
    git checkout -- .
    git pull --ff-only
    cd web-dashboard && npm run build
    cargo build --release -j 2 -p imc-cli
    ./target/release/imc install
    rm -rf target/release/incremental

release *ARGS:
    ./scripts/release-macos.sh {{ ARGS }}

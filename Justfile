default: update

# Cap nhat ma nguon (giu thay doi do dang bang autostash), build dashboard va cai dat binary
update:
    git pull --rebase --autostash
    cd web-dashboard && npm run build
    cargo build --release -j 2 -p imc-cli
    ./target/release/imc install
    rm -rf target/release/incremental

# Cap nhat sach (huy toan bo sua doi cuc bo, dong bo theo remote)
clean-update:
    git checkout -- .
    git pull --ff-only
    cd web-dashboard && npm run build
    cargo build --release -j 2 -p imc-cli
    ./target/release/imc install
    rm -rf target/release/incremental

# Tao ban release GitHub qua scripts/release-macos.sh
release *ARGS:
    ./scripts/release-macos.sh {{ ARGS }}

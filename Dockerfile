# cfbest-cli 的容器镜像：与图形界面共用同一套内核，但不需要窗口。
#
# cfbest-cli 本身不碰图形界面，而 gpui 在上游是必需依赖，Linux 上编译它会连带
# 要求 X11/Wayland 的开发库。这里只为 cfbest-cli 构建：去掉 gpui，并补上 tokio
# 的 io-util 特性（此前是 gpui 间接启用的），运行行为完全不变。
#
# 等 gpui 改为可选依赖后，下面几行 sed 就可以删掉。

FROM rust:1.96-bookworm AS build
WORKDIR /src

COPY Cargo.toml Cargo.lock build.rs ./
COPY src ./src

RUN sed -i '/^gpui = /d' Cargo.toml \
 && sed -i '/runtime_shaders/d' Cargo.toml \
 && sed -i 's/"macros"\]/"macros", "io-util"]/' Cargo.toml \
 && cargo build --release --bin cfbest-cli

FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/*

COPY --from=build /src/target/release/cfbest-cli /usr/local/bin/cfbest-cli

# 数据目录：设置、段池、订阅缓存、每日进度
ENV XDG_DATA_HOME=/data
VOLUME ["/data"]

ENTRYPOINT ["cfbest-cli"]
CMD ["--help"]

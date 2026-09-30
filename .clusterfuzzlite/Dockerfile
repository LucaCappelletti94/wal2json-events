FROM gcr.io/oss-fuzz-base/base-builder-rust
COPY . $SRC/wal2json-events
WORKDIR $SRC/wal2json-events
COPY ./.clusterfuzzlite/build.sh $SRC/

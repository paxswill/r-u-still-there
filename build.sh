#!/bin/sh

set -e
set -x

armv6 () {
	echo "Building and packaging ARMv6 (hard float)"
	TARGET_CFLAGS="-march=armv6+fp" cross build \
		--release \
		--target arm-unknown-linux-gnueabihf
	CARGO_TARGET_DIR="./target" cargo deb \
		--no-build \
		--no-strip \
		--variant v6 \
		--target arm-unknown-linux-gnueabihf \
		--output ./	
}

armv7() {
	echo "Building and packaging ARMv7 (hard float)"
	# cross 0.21 has *ancient* versions fo GCC for the
	# armv7-unknown-linux-gnueabihf docker image (4.6.2) so we need to use an
	# older way to use neon instructions. If/when the images are upgraded, just
	# `-march=armv7-a+simd` can be used.
	TARGET_CFLAGS="-march=armv7-a -mfpu=neon" cross build \
		--release \
		--target armv7-unknown-linux-gnueabihf \
		--features mozjpeg_simd
	CARGO_TARGET_DIR="./target" cargo deb \
		--no-build \
		--no-strip \
		--variant v7 \
		--target armv7-unknown-linux-gnueabihf \
		--output ./	
}

armv8() {
	echo "Building and packaging 64-bit ARM"
	cross build \
		--release \
		--target aarch64-unknown-linux-gnu \
		--features mozjpeg_simd
	CARGO_TARGET_DIR="./target" cargo deb \
	cargo deb \
		--no-build \
		--no-strip \
		--target aarch64-unknown-linux-gnu \
		--output ./	
}

# Later I should add a way to just build one arch, but for now build them all
armv6
armv7
armv8


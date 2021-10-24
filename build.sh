#!/bin/sh

set -e
set -x

armv6 () {
	echo "Building and packaging ARMv6 (hard float)"
	TARGET_CLFAGS="-march=armv6+fp" cross build \
		--release \
		--target arm-unknown-linux-gnueabihf
	cargo deb \
		--no-build \
		--no-strip \
		--variant v6 \
		--target arm-unknown-linux-gnueabihf \
		--output ./	
}

armv7() {
	echo "Building and packaging ARMv7 (hard float)"
	TARGET_CLFAGS="-march=armv7-a+simd" cross build \
		--release \
		--target armv7-unknown-linux-gnueabihf \
		--feature mozjpeg_simd
	cargo deb \
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
		--feature mozjpeg_simd
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


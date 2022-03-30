all: build

TEST_WORKDIR_PREFIX ?= "/tmp"
DOCKER ?= "true"

current_dir := $(shell dirname $(realpath $(firstword $(MAKEFILE_LIST))))
ARCH := $(shell uname -p)

env_go_path := $(shell go env GOPATH 2> /dev/null)
go_path := $(if $(env_go_path),$(env_go_path),"$(HOME)/go")

# Set the env DIND_CACHE_DIR to specify a cache directory for
# docker-in-docker container, used to cache data for docker pull,
# then mitigate the impact of docker hub rate limit, for example:
# env DIND_CACHE_DIR=/path/to/host/var-lib-docker make docker-nydusify-smoke
dind_cache_mount := $(if $(DIND_CACHE_DIR),-v $(DIND_CACHE_DIR):/var/lib/docker,)

FUSEDEV_COMMON = --target-dir target-fusedev --features=fusedev --release
VIRIOFS_COMMON = --target-dir target-virtiofs --features=virtiofs --release

# Functions

# Func: build golang target in docker
# Args:
#   $(1): The path where go build a golang project
#   $(2): How to build the golang project
define build_golang
	echo "Building target $@ by invoking: $(2)"
	if [ $(DOCKER) = "true" ]; then
		docker run --rm -v ${go_path}:/go -v ${current_dir}:/nydus-rs --workdir /nydus-rs/$(1) golang:1.17 $(2)
	else
		$(2) -C $(1)
	fi
endef

# Build nydus respecting different features
# $(1) is the specified feature. [fusedev, virtiofs]
define build_nydus
	cargo build --features=$(1) --target-dir target-$(1) $(CARGO_BUILD_FLAGS)
endef

define static_check
	# Cargo will skip checking if it is already checked
	cargo clippy --features=$(1) --workspace --bins --tests --target-dir target-$(1) -- -Dwarnings
endef

.PHONY: all .release_version .format .musl_target build release static-release fusedev-release virtiofs-release virtiofs fusedev

.release_version:
	$(eval CARGO_BUILD_FLAGS += --release)

.format:
	cargo fmt -- --check

.musl_target:
	$(eval CARGO_BUILD_FLAGS += --target ${ARCH}-unknown-linux-musl)

# Targets that are exposed to developers and users.
build: .format fusedev virtiofs
release: .format .release_version fusedev virtiofs
static-release: .musl_target .format .release_version fusedev virtiofs
fusedev-release: .format .release_version fusedev
virtiofs-release: .format .release_version virtiofs

virtiofs:
	# TODO: switch to --out-dir when it moves to stable
	# For now we build with separate target directories
	$(call build_nydus,$@,$@)
	$(call static_check,$@,target-$@)

fusedev:
	$(call build_nydus,$@,$@)
	$(call static_check,$@,target-$@)

clean:
	cargo clean
	cargo clean --target-dir target-virtiofs
	cargo clean --target-dir target-fusedev

# If virtiofs test must be performed, only run binary part
# Use same traget to avoid re-compile for differnt targets like gnu and musl
ut:
	TEST_WORKDIR_PREFIX=$(TEST_WORKDIR_PREFIX) RUST_BACKTRACE=1 cargo test --workspace $(FUSEDEV_COMMON) -- --skip integration --nocapture --test-threads=8
ifdef NYDUS_TEST_VIRTIOFS
# If virtiofs test must be performed, only run binary part since other package is not affected by feature - virtiofs
# Use same traget to avoid re-compile for differnt targets like gnu and musl
	RUST_BACKTRACE=1 cargo test $(VIRIOFS_COMMON) --bin nydusd -- --nocapture --test-threads=8
endif

CARGO_BUILD_GEARS = -v ~/.ssh/id_rsa:/root/.ssh/id_rsa -v ~/.cargo/git:/root/.cargo/git -v ~/.cargo/registry:/root/.cargo/registry

CARGO = $(shell which cargo)
SUDO = $(shell which sudo)

docker-static:
	docker build -t nydus-rs-static --build-arg ARCH=${ARCH} misc/musl-static
	docker run --rm ${CARGO_BUILD_GEARS} --workdir /nydus-rs -v ${current_dir}:/nydus-rs nydus-rs-static

# Run smoke test including general integration tests and unit tests in container.
# Nydus binaries should already be prepared.
smoke: ut
	# No need to involve `clippy check` here as build from target `virtiofs` or `fusedev` always does so.
	# TODO: Put each test function into separated rs file.
	$(SUDO) TEST_WORKDIR_PREFIX=$(TEST_WORKDIR_PREFIX) $(CARGO) test --test '*' $(FUSEDEV_COMMON) -- --nocapture --test-threads=8

docker-nydus-smoke:
	docker build -t nydus-smoke --build-arg ARCH=${ARCH} misc/nydus-smoke
	docker run --rm --privileged ${CARGO_BUILD_GEARS} \
		-e TEST_WORKDIR_PREFIX=$(TEST_WORKDIR_PREFIX) \
		-v ~/.cargo:/root/.cargo \
		-v $(TEST_WORKDIR_PREFIX) \
		-v ${current_dir}:/nydus-rs \
		nydus-smoke

NYDUSIFY_PATH = contrib/nydusify
# TODO: Nydusify smoke has to be time consuming for a while since it relies on musl nydusd and nydus-image.
# So musl compliation must be involved.
# And docker-in-docker deployment invovles image buiding?
docker-nydusify-smoke: docker-static
	$(call build_golang,$(NYDUSIFY_PATH),make build-smoke)
	docker build -t nydusify-smoke misc/nydusify-smoke
	docker run --rm --privileged \
		-e BACKEND_TYPE=$(BACKEND_TYPE) \
		-e BACKEND_CONFIG=$(BACKEND_CONFIG) \
		-v $(current_dir):/nydus-rs $(dind_cache_mount) nydusify-smoke TestSmoke

docker-nydusify-image-test: docker-static
	$(call build_golang,$(NYDUSIFY_PATH),make build-smoke)
	docker build -t nydusify-smoke misc/nydusify-smoke
	docker run --rm --privileged \
		-e BACKEND_TYPE=$(BACKEND_TYPE) \
		-e BACKEND_CONFIG=$(BACKEND_CONFIG) \
		-v $(current_dir):/nydus-rs $(dind_cache_mount) nydusify-smoke TestDockerHubImage

docker-smoke: docker-nydus-smoke docker-nydusify-smoke

nydusify:
	$(call build_golang,${NYDUSIFY_PATH},make)

nydusify-test:
	$(call build_golang,${NYDUSIFY_PATH},make test)

nydusify-static:
	$(call build_golang,${NYDUSIFY_PATH},make static-release)

CTR-REMOTE_PATH = contrib/ctr-remote
ctr-remote:
	$(call build_golang,${CTR-REMOTE_PATH},make)

ctr-remote-test:
	$(call build_golang,${CTR-REMOTE_PATH},make test)

ctr-remote-static:
	$(call build_golang,${CTR-REMOTE_PATH},make static-release)

NYDUS-OVERLAYFS_PATH = contrib/nydus-overlayfs
nydus-overlayfs:
	$(call build_golang,${NYDUS-OVERLAYFS_PATH},make)

nydus-overlayfs-test:
	$(call build_golang,${NYDUS-OVERLAYFS_PATH},make test)

nydus-overlayfs-static:
	$(call build_golang,${NYDUS-OVERLAYFS_PATH},make static-release)

DOCKER-GRAPHDRIVER_PATH = contrib/docker-nydus-graphdriver
docker-nydus-graphdriver:
	$(call build_golang,${DOCKER-GRAPHDRIVER_PATH},make)

docker-nydus-graphdriver-test:
	$(call build_golang,${DOCKER-GRAPHDRIVER_PATH},make test)

docker-nydus-graphdriver-static:
	$(call build_golang,${DOCKER-GRAPHDRIVER_PATH},make static-release)

# Run integration smoke test in docker-in-docker container. It requires some special settings,
# refer to `misc/example/README.md` for details.
all-static-release: docker-static all-contrib-static-release

all-contrib-static-release: nydusify-static ctr-remote-static \
			    nydus-overlayfs-static docker-nydus-graphdriver-static

all-contrib-test: nydusify-test ctr-remote-test \
				nydus-overlayfs-test docker-nydus-graphdriver-test

# https://www.gnu.org/software/make/manual/html_node/One-Shell.html
.ONESHELL:
docker-example: all-static-release
	cp ${current_dir}/target-fusedev/${ARCH}-unknown-linux-musl/release/nydusd misc/example
	cp ${current_dir}/target-fusedev/${ARCH}-unknown-linux-musl/release/nydus-image misc/example
	cp contrib/nydusify/cmd/nydusify misc/example
	docker build -t nydus-rs-example misc/example
	@cid=$(shell docker run --rm -t -d --privileged $(dind_cache_mount) nydus-rs-example)
	@docker exec $$cid /run.sh
	@EXIT_CODE=$$?
	@docker rm -f $$cid
	@exit $$EXIT_CODE

{
  description = "BigBrother Discord Archiver";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";

    crane.url = "github:ipetkov/crane";

    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, fenix, crane, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
        };

        rust-toolchain = fenix.packages.${system}.default.toolchain;
        craneLib = (crane.mkLib pkgs).overrideToolchain rust-toolchain;

        project = pkgs.lib.importTOML ./Cargo.toml;

        commonArgs = {
          src = craneLib.cleanCargoSource ./.;
          strictDeps = true;

          nativeBuildInputs = with pkgs; [
            pkg-config
            openssl
            cmake
            clang
            mold
          ];

          buildInputs = with pkgs; [
            openssl
          ] ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [
            pkgs.libiconv
            pkgs.darwin.apple_sdk.frameworks.Security
            pkgs.darwin.apple_sdk.frameworks.SystemConfiguration
          ];
        };

        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        bigbrother = craneLib.buildPackage (commonArgs // {
          inherit cargoArtifacts;
          pname = project.package.name;
          version = project.package.version;
          doCheck = false;
        });
      in
      {
        packages.default = bigbrother;

        devShells.default = pkgs.mkShell {
          inputsFrom = [ bigbrother ];
          packages = with pkgs; [
            rust-analyzer
            rustfmt
            clippy
          ];
        };
      }) // {
        nixosModules.default = { config, pkgs, lib, ... }:
          let
            cfg = config.services.bigbrother;

            toEnv = val: if builtins.isBool val then (if val then "true" else "false") else toString val;
          in
          {
            options.services.bigbrother = {
              enable = lib.mkEnableOption "BigBrother Discord Archiver";

              package = lib.mkOption {
                type = lib.types.package;
                default = self.packages.${pkgs.system}.default;
                description = "The package to use.";
              };

              token = lib.mkOption {
                type = lib.types.str;
                description = "The Discord Bot Token.";
                example = "MTAw...";
              };

              dataDir = lib.mkOption {
                type = lib.types.str;
                default = "/var/lib/bigbrother";
                description = "Directory to store archives and assets.";
              };

              settings = {
                network = {
                  timeout = lib.mkOption { type = lib.types.int; default = 120; };
                  downloadConcurrency = lib.mkOption { type = lib.types.int; default = 10; };
                };
                catchup = {
                  messagesPerRequest = lib.mkOption { type = lib.types.int; default = 100; };
                  writeBatchSize = lib.mkOption { type = lib.types.int; default = 1000; };
                  channelConcurrency = lib.mkOption { type = lib.types.int; default = 4; };
                };
                metadata = {
                  memberFetchLimit = lib.mkOption { type = lib.types.int; default = 1000; };
                };
                storage = {
                  autoflushInterval = lib.mkOption { type = lib.types.int; default = 60000; };
                };
              };
            };

            config = lib.mkIf cfg.enable {

              users.users.bigbrother = {
                isSystemUser = true;
                group = "bigbrother";
                description = "BigBrother Service User";
              };
              users.groups.bigbrother = {};

              systemd.services.bigbrother = {
                description = "BigBrother Discord Archiver";
                after = [ "network-online.target" ];
                wants = [ "network-online.target" ];
                wantedBy = [ "multi-user.target" ];

                environment = {
                  BIGBROTHER_DISCORD_TOKEN = cfg.token;
                  BIGBROTHER_DATA_PATH = cfg.dataDir;

                  BIGBROTHER_NETWORK_TIMEOUT = toEnv cfg.settings.network.timeout;
                  BIGBROTHER_NETWORK_DOWNLOAD_CONCURRENCY_LIMIT = toEnv cfg.settings.network.downloadConcurrency;
                  BIGBROTHER_CATCHUP_MESSAGES_PER_REQUEST = toEnv cfg.settings.catchup.messagesPerRequest;
                  BIGBROTHER_CATCHUP_WRITE_BATCH_SIZE = toEnv cfg.settings.catchup.writeBatchSize;
                  BIGBROTHER_CATCHUP_CHANNEL_CONCURRENCY = toEnv cfg.settings.catchup.channelConcurrency;
                  BIGBROTHER_METADATA_MEMBER_FETCH_LIMIT = toEnv cfg.settings.metadata.memberFetchLimit;
                  BIGBROTHER_STORAGE_AUTOFLUSH_INTERVAL_MS = toEnv cfg.settings.storage.autoflushInterval;

                  RUST_LOG = "info,bigbrother=info";
                };

                serviceConfig = {
                  ExecStart = "${cfg.package}/bin/BigBrother";

                  User = "bigbrother";
                  Group = "bigbrother";

                  StateDirectory = "bigbrother";
                  WorkingDirectory = cfg.dataDir;

                  CapabilityBoundingSet = "";
                  ProcSubset = "pid";
                  ProtectProc = "invisible";
                  NoNewPrivileges = true;
                  ProtectSystem = "strict";
                  ProtectHome = true;
                  PrivateTmp = true;
                  PrivateDevices = true;
                  PrivateUsers = true;
                  ProtectHostname = true;
                  ProtectClock = true;
                  ProtectKernelTunables = true;
                  ProtectKernelModules = true;
                  ProtectKernelLogs = true;
                  ProtectControlGroups = true;
                  RestrictAddressFamilies = [ "AF_INET" "AF_INET6" ];
                  RestrictNamespaces = true;
                  LockPersonality = true;
                  MemoryDenyWriteExecute = true;
                  RestrictRealtime = true;
                  RestrictSUIDSGID = true;
                  RemoveIPC = true;

                  Restart = "on-failure";
                  RestartSec = "5s";
                };

                preStart = ''
                  touch ${cfg.dataDir}/config.toml
                '';
              };
            };
          };
      };
}
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import "forge-std/Script.sol";
// import "openzeppelin-foundry-upgrades/Upgrades.sol";
import {Upgrades} from "openzeppelin-foundry-upgrades/Upgrades.sol";
import "openzeppelin-foundry-upgrades/Options.sol";
import "@openzeppelin/contracts/utils/Strings.sol";
import "../contracts/BridgeCommittee.sol";
import "../contracts/BridgeVault.sol";
import "../contracts/BridgeConfig.sol";
import "../contracts/BridgeLimiter.sol";
import "../contracts/IotaBridge.sol";
import "../test/mocks/MockTokens.sol";

contract DeployBridge is Script {
    function run() external {
        uint256 deployerPrivateKey = vm.envUint("PRIVATE_KEY");
        vm.startBroadcast(deployerPrivateKey);
        string memory chainID = Strings.toString(block.chainid);
        bytes32 chainIDHash = keccak256(abi.encode(chainID));
        bool isLocal = chainIDHash != keccak256(abi.encode("11155111"))
            && chainIDHash != keccak256(abi.encode("1"));
        string memory root = vm.projectRoot();
        string memory path = string.concat(root, "/deploy_configs/", chainID, ".json");
        // If this is local deployment, we override the path if OVERRIDE_CONFIG_PATH is set.
        // This is useful in integration tests where config path is not fixed.
        if (isLocal) {
            path = vm.envOr("OVERRIDE_CONFIG_PATH", path);
        }

        console.log("config path: ", path);
        string memory json = vm.readFile(path);
        bytes memory bytesJson = vm.parseJson(json);
        DeployConfig memory deployConfig = abi.decode(bytesJson, (DeployConfig));

        // if deploying to local network, deploy mock tokens
        if (isLocal) {
            console.log("Deploying mock tokens for local network");
            // deploy WETH
            deployConfig.WETH = address(new WETH());

            // deploy mock tokens
            MockWBTC wBTC = new MockWBTC();
            MockUSDC USDC = new MockUSDC();
            MockUSDT USDT = new MockUSDT();
            MockKA KA = new MockKA();
            console.log("[Deployed] KA:", address(KA));

            // update deployConfig with mock addresses
            deployConfig.supportedTokens = new address[](5);
            // In BridgeConfig.sol `supportedTokens is shifted by one
            // and the first token is IOTA.
            deployConfig.supportedTokens[0] = address(0);
            deployConfig.supportedTokens[1] = address(wBTC);
            deployConfig.supportedTokens[2] = deployConfig.WETH;
            deployConfig.supportedTokens[3] = address(USDC);
            deployConfig.supportedTokens[4] = address(USDT);
        }

        // TODO: validate config values before deploying

        // convert supported chains from uint256 to uint8[]
        uint8[] memory supportedChainIDs = new uint8[](deployConfig.supportedChainIDs.length);
        for (uint256 i; i < deployConfig.supportedChainIDs.length; i++) {
            supportedChainIDs[i] = uint8(deployConfig.supportedChainIDs[i]);
        }

        // deploy bridge config
        // price of IOTA (id = 0) should not be included in tokenPrices
        require(
            deployConfig.supportedTokens.length == deployConfig.tokenPrices.length,
            "supportedTokens.length + 1 != tokenPrices.length"
        );

        // deploy Bridge Committee ===================================================================

        // convert committeeMembers stake from uint256 to uint16[]
        uint16[] memory committeeMemberStake =
            new uint16[](deployConfig.committeeMemberStake.length);
        for (uint256 i; i < deployConfig.committeeMemberStake.length; i++) {
            committeeMemberStake[i] = uint16(deployConfig.committeeMemberStake[i]);
        }

        Options memory opts;
        opts.unsafeSkipAllChecks = true;

        address bridgeCommittee = Upgrades.deployUUPSProxy(
            "BridgeCommittee.sol",
            abi.encodeCall(
                BridgeCommittee.initialize,
                (
                    deployConfig.committeeMembers,
                    committeeMemberStake,
                    uint16(deployConfig.minCommitteeStakeRequired)
                )
            ),
            opts
        );

        // deploy bridge config =====================================================================

        // convert token prices from uint256 to uint64
        uint64[] memory tokenPrices = new uint64[](deployConfig.tokenPrices.length);
        for (uint256 i; i < deployConfig.tokenPrices.length; i++) {
            tokenPrices[i] = uint64(deployConfig.tokenPrices[i]);
        }

        address bridgeConfig = Upgrades.deployUUPSProxy(
            "BridgeConfig.sol",
            abi.encodeCall(
                BridgeConfig.initialize,
                (
                    address(bridgeCommittee),
                    uint8(deployConfig.sourceChainId),
                    deployConfig.supportedTokens,
                    tokenPrices,
                    supportedChainIDs
                )
            ),
            opts
        );

        // initialize config in the bridge committee
        BridgeCommittee(bridgeCommittee).initializeConfig(address(bridgeConfig));

        // deploy vault =============================================================================

        BridgeVault vault = new BridgeVault(deployConfig.WETH);

        // deploy limiter ===========================================================================

        // convert chain limits from uint256 to uint64[]
        uint64[] memory chainLimits =
            new uint64[](deployConfig.supportedChainLimitsInDollars.length);
        for (uint256 i; i < deployConfig.supportedChainLimitsInDollars.length; i++) {
            chainLimits[i] = uint64(deployConfig.supportedChainLimitsInDollars[i]);
        }

        address limiter = Upgrades.deployUUPSProxy(
            "BridgeLimiter.sol",
            abi.encodeCall(
                BridgeLimiter.initialize, (bridgeCommittee, supportedChainIDs, chainLimits)
            ),
            opts
        );

        uint8[] memory _destinationChains = new uint8[](1);
        _destinationChains[0] = 1;

        // deploy IOTA Bridge ========================================================================

        address iotaBridge = Upgrades.deployUUPSProxy(
            "IotaBridge.sol",
            abi.encodeCall(IotaBridge.initialize, (bridgeCommittee, address(vault), limiter)),
            opts
        );

        // transfer vault ownership to bridge
        vault.transferOwnership(iotaBridge);
        // transfer limiter ownership to bridge
        BridgeLimiter instance = BridgeLimiter(limiter);
        instance.transferOwnership(iotaBridge);

        // print deployed addresses for post deployment setup
        console.log("[Deployed] BridgeConfig:", bridgeConfig);
        console.log("[Deployed] IotaBridge:", iotaBridge);
        console.log("[Deployed] BridgeLimiter:", limiter);
        console.log("[Deployed] BridgeCommittee:", bridgeCommittee);
        console.log("[Deployed] BridgeVault:", address(vault));
        console.log("[Deployed] BTC:", BridgeConfig(bridgeConfig).tokenAddressOf(1));
        console.log("[Deployed] ETH:", BridgeConfig(bridgeConfig).tokenAddressOf(2));
        console.log("[Deployed] USDC:", BridgeConfig(bridgeConfig).tokenAddressOf(3));
        console.log("[Deployed] USDT:", BridgeConfig(bridgeConfig).tokenAddressOf(4));

        vm.stopBroadcast();
    }

    // used to ignore for forge coverage
    function testSkip() public {}
}

/// check the following for guidelines on updating deploy_configs and references:
/// https://book.getfoundry.sh/cheatcodes/parse-json
struct DeployConfig {
    uint256[] committeeMemberStake;
    address[] committeeMembers;
    uint256 minCommitteeStakeRequired;
    uint256 sourceChainId;
    uint256[] supportedChainIDs;
    uint256[] supportedChainLimitsInDollars;
    address[] supportedTokens;
    uint256[] tokenPrices;
    address WETH;
}

import { OFT } from "@layerzerolabs/lz-iotal1-oft-sdk-v2";
import { SDK, validateTransaction } from "@layerzerolabs/lz-iotal1-sdk-v2";
import { Transaction } from "@iota/iota-sdk/transactions";
import { IotaClient } from '@iota/iota-sdk/client';
import { Stage } from "@layerzerolabs/lz-definitions"
import { toBase64 } from '@iota/bcs';
import { Options } from '@layerzerolabs/lz-v2-utilities';

const iotaClient = new IotaClient({
    url: 'https://indexer.testnet.iota.cafe',
});

// Initialize LayerZero protocol SDK
const protocolSDK = new SDK({
    client: iotaClient,
    stage: Stage.TESTNET,
});

const oftPackageId = '0x7d8f364f6540c862d8de6ce5eccc4217729abc164e5262c6965864beec28dcca'

// Create OFT instance (with optional parameters for convenience)
const oft = new OFT(protocolSDK, oftPackageId);

const senderAddress = '0xa1a97d20bbad79e2ac89f215a3b3c4f2ff9a1aa3cc26e529bde6e7bc5500d610'
// Prepare send parameters
const sendParam = {
    dstEid: 40423, // Destination endpoint ID
    to: (() => { const arr = new Uint8Array(32); arr.set(Buffer.from(senderAddress.slice(2), 'hex')); return arr; })(), // Recipient address as Uint8Array (32 bytes)
    amountLd: 10n, // Amount in local decimals
    minAmountLd: 9n, // Minimum amount (slippage protection)
    extraOptions: Options.newOptions().addExecutorLzReceiveOption(1, 0).toBytes(),// new Uint8Array(0), // LayerZero execution options
    composeMsg: new Uint8Array(0), // Optional compose message
    oftCmd: new Uint8Array(0), // Optional OFT command (unused in default OFT)
};

// Quote the transfer fees
// const messagingFee = await oft.quoteSend(
//     senderAddress,
//     sendParam,
//     false, // payInZro: false = pay in native token
// );

// Execute the transfer
const tx = new Transaction();

// Split coins from sender's wallet
// const coin = await oft.splitCoinMoveCall(tx, senderAddress, sendParam.amountLd);
const coin = await oft.splitCoinMoveCall(tx, senderAddress, BigInt(20));

try {

    // Send the tokens
    await oft.sendMoveCall(
        tx,
        senderAddress,
        sendParam,
        coin,
        // messagingFee.nativeFee,
        // messagingFee.zroFee,
        1000000000,
        0,
        senderAddress, // refund address
    );


    // Transfer any remaining coins back to sender
    tx.transferObjects([coin], senderAddress);
    tx.setSender(senderAddress);
    tx.setGasBudget(1000000000);
    // let bytes = await tx.build({ client: iotaClient });
    // console.log(bytes)
    let transactionBytes = toBase64(await tx.build({ client: iotaClient }));
    console.log(transactionBytes)
    let dryRun = await iotaClient.dryRunTransactionBlock({ transactionBlock: transactionBytes })
    console.log(dryRun.effects.status)
} catch (e) {
    console.error(e)
}

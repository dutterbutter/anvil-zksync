import { expect } from "chai";
import * as hre from "hardhat";
import { deployContract, getTestProvider } from "../helpers/utils";
import { Wallet } from "zksync-ethers";
import { RichAccounts } from "../helpers/constants";
import { ethers } from "ethers";
import { Deployer } from "@matterlabs/hardhat-zksync-deploy";

const provider = getTestProvider();

describe("evm_setAccountNonce", function () {
  it("Should update the nonce of an account", async function () {
    // Arrange
    const userWallet = Wallet.createRandom().connect(provider);
    const newNonce = 42;

    // Act
    await provider.send("evm_setAccountNonce", [userWallet.address, ethers.toBeHex(newNonce)]);

    // Assert
    const nonce = await userWallet.getNonce();
    expect(nonce).to.equal(newNonce);
  });
});

describe("evm_mine", function () {
  it("Should mine one block", async function () {
    // Arrange
    const startingBlock = await provider.getBlock("latest");

    // Act
    await provider.send("evm_mine", []);

    // Assert
    const latestBlock = await provider.getBlock("latest");
    expect(latestBlock.number).to.equal(startingBlock.number + 1);
  });
});

describe("evm_increaseTime", function () {
  it("Should increase current timestamp of the node", async function () {
    // Arrange
    const timeIncreaseInSeconds = 13;
    const wallet = new Wallet(RichAccounts[0].PrivateKey, provider);
    const userWallet = Wallet.createRandom().connect(provider);
    let expectedTimestamp: number = await provider.send("config_getCurrentTimestamp", []);
    expectedTimestamp += timeIncreaseInSeconds;

    // Act
    await provider.send("evm_increaseTime", [timeIncreaseInSeconds]);

    const txResponse = await wallet.sendTransaction({
      to: userWallet.address,
      value: ethers.parseEther("0.1"),
    });
    await txResponse.wait();
    expectedTimestamp += 2; // New transaction will add two blocks

    // Assert
    const newBlockTimestamp = (await provider.getBlock("latest")).timestamp;
    expect(newBlockTimestamp).to.equal(expectedTimestamp);
  });
});

describe("evm_setNextBlockTimestamp", function () {
  it("Should set current timestamp of the node to specific value", async function () {
    // Arrange
    const timeIncreaseInMS = 123;
    let expectedTimestamp: number = await provider.send("config_getCurrentTimestamp", []);
    expectedTimestamp += timeIncreaseInMS;
    const wallet = new Wallet(RichAccounts[0].PrivateKey, provider);
    const userWallet = Wallet.createRandom().connect(provider);

    // Act
    await provider.send("evm_setNextBlockTimestamp", [expectedTimestamp]);

    const txResponse = await wallet.sendTransaction({
      to: userWallet.address,
      value: ethers.parseEther("0.1"),
    });
    await txResponse.wait();
    expectedTimestamp += 1; // After executing a transaction, the node puts it into a block and increases its current timestamp

    // Assert
    const newBlockTimestamp = (await provider.getBlock("latest")).timestamp;
    expect(newBlockTimestamp).to.equal(expectedTimestamp);
  });
});

describe("evm_setTime", function () {
  it("Should set current timestamp of the node to specific value", async function () {
    // Arrange
    const timeIncreaseInMS = 123;
    let expectedTimestamp: number = await provider.send("config_getCurrentTimestamp", []);
    expectedTimestamp += timeIncreaseInMS;
    const wallet = new Wallet(RichAccounts[0].PrivateKey, provider);
    const userWallet = Wallet.createRandom().connect(provider);

    // Act
    await provider.send("evm_setTime", [expectedTimestamp]);

    const txResponse = await wallet.sendTransaction({
      to: userWallet.address,
      value: ethers.parseEther("0.1"),
    });
    await txResponse.wait();
    expectedTimestamp += 2; // New transaction will add two blocks

    // Assert
    const newBlockTimestamp = (await provider.getBlock("latest")).timestamp;
    expect(newBlockTimestamp).to.equal(expectedTimestamp);
  });
});

describe("evm_snapshot", function () {
  it("Should return incrementing snapshot ids", async function () {
    const wallet = new Wallet(RichAccounts[0].PrivateKey);
    const deployer = new Deployer(hre, wallet);
    const greeter = await deployContract(deployer, "Greeter", ["Hi"]);
    expect(await greeter.greet()).to.eq("Hi");

    // Act
    const snapshotId1: string = await provider.send("evm_snapshot", []);
    const snapshotId2: string = await provider.send("evm_snapshot", []);

    // Assert
    expect(await greeter.greet()).to.eq("Hi");
    expect(BigInt(snapshotId2)).to.eq(BigInt(snapshotId1) + 1n);
  });
});

describe("evm_revert", function () {
  it("Should revert with correct snapshot id", async function () {
    const wallet = new Wallet(RichAccounts[0].PrivateKey);
    const deployer = new Deployer(hre, wallet);
    const greeter = await deployContract(deployer, "Greeter", ["Hi"]);
    expect(await greeter.greet()).to.eq("Hi");
    const snapshotId = await provider.send("evm_snapshot", []);
    const setGreetingTx = await greeter.setGreeting("Hola, mundo!");
    await setGreetingTx.wait();
    expect(await greeter.greet()).to.equal("Hola, mundo!");

    // Act
    const reverted: boolean = await provider.send("evm_revert", [snapshotId]);

    // Assert
    expect(await greeter.greet()).to.eq("Hi");
    expect(reverted).to.be.true;
  });
});

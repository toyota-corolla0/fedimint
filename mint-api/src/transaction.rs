use crate::{Amount, Coin, Coins, Encodable, FeeConsensus, PegInProof, TransactionId};
use bitcoin_hashes::Hash as BitcoinHash;
use musig::{PubKey, Sig};
use serde::{Deserialize, Serialize};
use std::io::Error;
use thiserror::Error;

#[derive(Debug, Clone, Eq, PartialEq, Hash, Deserialize, Serialize)]
pub struct Transaction {
    pub inputs: Vec<Input>,
    pub outputs: Vec<Output>,
    pub signature: Sig,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, Deserialize, Serialize)]
pub enum Input {
    // TODO: maybe treat every coin as a seperate input?
    Coins(Coins<Coin>),
    PegIn(PegInProof),
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, Deserialize, Serialize)]
pub enum Output {
    Coins(Coins<BlindToken>),
    PegOut(PegOut),
    // TODO: lightning integration goes here
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, Deserialize, Serialize)]
pub struct PegOut {
    pub recipient: bitcoin::Address,
    #[serde(with = "bitcoin::util::amount::serde::as_sat")]
    pub amount: bitcoin::Amount,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, Deserialize, Serialize)]
pub struct BlindToken(pub tbs::BlindedMessage);

#[derive(Debug, Clone, Eq, PartialEq, Hash, Deserialize, Serialize)]
pub struct OutPoint {
    pub txid: TransactionId,
    pub out_idx: usize,
}

/// Common properties of transaction in- and outputs
pub trait TransactionItem {
    /// The amount before fees represented by the in/output
    fn amount(&self) -> crate::Amount;

    /// The fee that will be charged for this in/output
    fn fee(&self, fee_consensus: &FeeConsensus) -> crate::Amount;
}

impl Input {
    // TODO: probably make this a single returned key once coins are separate inputs
    /// Returns an iterator over all the keys that need to sign the transaction for the input to
    /// be valid.
    fn authorization_keys<'a>(&'a self) -> Box<dyn Iterator<Item = &'a PubKey> + 'a> {
        match self {
            Input::Coins(coins) => Box::new(coins.iter().map(|(_, coin)| coin.spend_key())),
            Input::PegIn(proof) => Box::new(std::iter::once(proof.tweak_contract_key())),
        }
    }
}

impl TransactionItem for Input {
    fn amount(&self) -> Amount {
        match self {
            Input::Coins(coins) => coins.amount(),
            Input::PegIn(peg_in) => Amount::from_sat(peg_in.tx_output().value),
        }
    }

    fn fee(&self, fee_consensus: &FeeConsensus) -> Amount {
        match self {
            Input::Coins(coins) => fee_consensus.fee_coin_spend_abs * (coins.coins.len() as u64),
            Input::PegIn(_) => fee_consensus.fee_peg_in_abs,
        }
    }
}

impl TransactionItem for Output {
    fn amount(&self) -> Amount {
        match self {
            Output::Coins(coins) => coins.amount(),
            Output::PegOut(peg_out) => peg_out.amount.into(),
        }
    }

    fn fee(&self, fee_consensus: &FeeConsensus) -> Amount {
        match self {
            Output::Coins(coins) => fee_consensus.fee_coin_spend_abs * (coins.coins.len() as u64),
            Output::PegOut(_) => fee_consensus.fee_peg_out_abs,
        }
    }
}

impl Transaction {
    pub fn validate_funding(&self, fee_consensus: &FeeConsensus) -> Result<(), TransactionError> {
        let in_amount = self
            .inputs
            .iter()
            .map(TransactionItem::amount)
            .sum::<Amount>();
        let out_amount = self
            .outputs
            .iter()
            .map(TransactionItem::amount)
            .sum::<Amount>();
        let fee_amount = self
            .inputs
            .iter()
            .map(|input| input.fee(fee_consensus))
            .sum::<Amount>()
            + self
                .outputs
                .iter()
                .map(|output| output.fee(fee_consensus))
                .sum::<Amount>();

        if in_amount >= (out_amount + fee_amount) {
            Ok(())
        } else {
            Err(TransactionError::InsufficientlyFunded {
                inputs: in_amount,
                outputs: out_amount,
                fee: fee_amount,
            })
        }
    }

    /// Hash the transaction excluding the signature. This hash is what the signature inside the
    /// transaction commits to. To generate it without already having a signature use [tx_hash_from_parts].
    pub fn tx_hash(&self) -> TransactionId {
        Self::tx_hash_from_parts(&self.inputs, &self.outputs)
    }

    /// Generates the transaction hash without constructing the transaction (which would require a
    /// signature).
    pub fn tx_hash_from_parts(inputs: &[Input], outputs: &[Output]) -> TransactionId {
        let mut engine = TransactionId::engine();
        inputs
            .consensus_encode(&mut engine)
            .expect("write to hash engine can't fail");
        outputs
            .consensus_encode(&mut engine)
            .expect("write to hash engine can't fail");
        TransactionId::from_engine(engine)
    }

    pub fn validate_signature(&self) -> Result<(), TransactionError> {
        let public_keys = self
            .inputs
            .iter()
            .flat_map(|input| input.authorization_keys())
            .collect::<Vec<_>>();

        if musig::verify(
            self.tx_hash().into_inner(),
            self.signature.clone(),
            &public_keys,
        ) {
            Ok(())
        } else {
            Err(TransactionError::InvalidSignature)
        }
    }
}

impl Encodable for Input {
    fn consensus_encode<W: std::io::Write>(&self, mut writer: W) -> Result<usize, Error> {
        match self {
            Input::Coins(coins) => {
                writer.write_all(&[0x00])?;
                coins.consensus_encode(writer).map(|len| len + 1)
            }
            Input::PegIn(peg_in) => {
                writer.write_all(&[0x01])?;
                peg_in.consensus_encode(writer).map(|len| len + 1)
            }
        }
    }
}

impl Encodable for Output {
    fn consensus_encode<W: std::io::Write>(&self, mut writer: W) -> Result<usize, Error> {
        match self {
            Output::Coins(coins) => {
                writer.write_all(&[0x00])?;
                coins.consensus_encode(writer).map(|len| len + 1)
            }
            Output::PegOut(peg_out) => {
                writer.write_all(&[0x01])?;
                peg_out.consensus_encode(writer).map(|len| len + 1)
            }
        }
    }
}

impl Encodable for PegOut {
    fn consensus_encode<W: std::io::Write>(&self, mut writer: W) -> Result<usize, Error> {
        let mut len = 0;
        // TODO: for decode also encode network here or change address type to script
        len += self
            .recipient
            .script_pubkey()
            .consensus_encode(&mut writer)?;
        len += self.amount.consensus_encode(&mut writer)?;

        Ok(len)
    }
}

impl Encodable for BlindToken {
    fn consensus_encode<W: std::io::Write>(&self, mut writer: W) -> Result<usize, Error> {
        writer.write_all(&self.0.encode_compressed())?;
        Ok(48)
    }
}

#[derive(Debug, Error)]
pub enum TransactionError {
    #[error("The transaction is insufficiently funded (in={inputs}, out={outputs}, fee={fee})")]
    InsufficientlyFunded {
        inputs: Amount,
        outputs: Amount,
        fee: Amount,
    },
    #[error("The transaction's signature is invalid")]
    InvalidSignature,
}

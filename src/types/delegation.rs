//! NIP26
//!
//! <https://github.com/nostr-protocol/nips/blob/master/26.md>

use crate::{Error, Event, Id, PrivateKey, PublicKey, Signature};

use k256::schnorr::signature::Verifier;
use k256::sha2::{Digest, Sha256};
use serde_json::{json, Value};

use std::fmt;
use std::str::FromStr;

/// `NIP26` error
#[derive(Debug, thiserror::Error)]
pub enum DelegationError {
    /// Signing error
    #[error("Signing error")]
    SigningError,
    /// Signature error, such as verification error
    #[error(transparent)]
    Key(#[from] k256::ecdsa::Error),
    /// Invalid condition in conditions string
    #[error("Invalid condition in conditions string")]
    ConditionsParseInvalidCondition,
    /// Invalid condition, cannot parse expected number
    #[error("Invalid condition, cannot parse expected number")]
    ConditionsParseNumeric(#[from] std::num::ParseIntError),
    /// Conditions not satisfied
    #[error("Conditions not satisfied")]
    ConditionsValidation(#[from] ValidationError),
    /// Delegation tag json parse error
    #[error("Delegation tag json parse error")]
    DelegationTagParseJson(#[from] serde_json::Error),
    /// Delegation tag parse error
    #[error("Delegation tag parse error")]
    DelegationTagParse,
}

/// Tag validation errors
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ValidationError {
    /// Signature does not match
    #[error("Signature does not match")]
    InvalidSignature,
    /// Event kind does not match
    #[error("Event kind does not match")]
    InvalidKind,
    /// Creation time is earlier than validity period
    #[error("Creation time is earlier than validity period")]
    CreatedTooEarly,
    /// Creation time is later than validity period
    #[error("Creation time is later than validity period")]
    CreatedTooLate,
}

const DELEGATION_KEYWORD: &str = "delegation";

/// Compile the delegation token, of the form 'nostr:delegation:<pubkey of publisher (delegatee)>:<conditions query string>'
pub(crate) fn delegation_token(delegatee_pubkey: &PublicKey, conditions: &str) -> String {
    format!(
        "nostr:{}:{}:{}",
        DELEGATION_KEYWORD,
        delegatee_pubkey.as_hex_string(),
        conditions
    )
}

fn hash_256(input: &[u8]) -> Id {
    let mut hasher = Sha256::new();
    hasher.update(input);
    let id = hasher.finalize();
    Id(id.into())
}

/// Sign delegation.
/// See `DelegationTag::create` for more complete functionality.
fn sign_delegation(
    delegator_privkey: &PrivateKey,
    delegatee_pubkey: &PublicKey,
    conditions: String,
) -> Result<Signature, Error> {
    let unhashed_token: String = delegation_token(&delegatee_pubkey, &conditions);
    let hashed_token = hash_256(unhashed_token.as_bytes());
    match delegator_privkey.sign_id(hashed_token) {
        Err(_e) => Err(Error::DelegationError(DelegationError::SigningError)),
        Ok(sig) => Ok(sig),
    }
}

/// Verify delegation signature
pub(crate) fn verify_delegation_signature(
    delegator_pubkey: &PublicKey,
    signature: &Signature,
    delegatee_pubkey: &PublicKey,
    conditions: String,
) -> Result<(), Error> {
    let unhashed_token: String = delegation_token(&delegatee_pubkey, &conditions);
    let _verify_res = delegator_pubkey
        .0
        .verify(unhashed_token.as_bytes(), signature)?;
    Ok(())
}

/// Delegation tag, as defined in NIP-26
#[derive(Clone, Debug)]
pub struct DelegationTag {
    delegator_pubkey: PublicKey,
    conditions: String,
    signature: Signature,
}

impl DelegationTag {
    /// Accessor for delegator public key
    pub fn get_delegator_pubkey(&self) -> PublicKey {
        self.delegator_pubkey
    }

    /// Accessor for conditions
    pub(crate) fn get_conditions(&self) -> String {
        self.conditions.clone()
    }

    /// Accessor for signature
    pub fn get_signature(&self) -> Signature {
        self.signature
    }

    /// Create a delegation tag (including the signature).
    /// See also validate().
    pub fn create(
        delegator_privkey: &PrivateKey,
        delegatee_pubkey: &PublicKey,
        conditions_string: &str,
    ) -> Result<Self, Error> {
        let signature = sign_delegation(
            delegator_privkey,
            delegatee_pubkey,
            conditions_string.to_string(),
        )?;
        let conditions_struct = Conditions::from_str(conditions_string)?;
        let conditions = conditions_struct.to_string();
        Ok(Self {
            delegator_pubkey: delegator_privkey.public_key(),
            conditions,
            signature,
        })
    }

    /// Validate a delegation tag, check signature and conditions.
    pub fn validate(
        &self,
        delegatee_pubkey: &PublicKey,
        event_properties: &EventProperties,
    ) -> Result<(), Error> {
        // verify signature
        if let Err(_e) = verify_delegation_signature(
            &self.get_delegator_pubkey(),
            &self.get_signature(),
            delegatee_pubkey,
            self.get_conditions(),
        ) {
            return Err(Error::DelegationError(
                DelegationError::ConditionsValidation(ValidationError::InvalidSignature),
            ));
        }

        // validate conditions
        let conditions_struct = Conditions::from_str(&self.conditions)?;
        let _eval_res = conditions_struct
            .evaluate(event_properties)
            .map_err(|e| DelegationError::ConditionsValidation(e))?;
        Ok(())
    }

    /// Convert to JSON string.
    pub fn as_json(&self) -> String {
        let tag = json!([
            DELEGATION_KEYWORD,
            self.delegator_pubkey.as_hex_string(),
            self.conditions,
            self.signature.as_hex_string(),
        ]);
        tag.to_string()
    }

    /// Parse from a JSON string
    pub fn from_json(s: &str) -> Result<Self, Error> {
        let v = serde_json::from_str::<Value>(s)?;
        let arr = match v.as_array() {
            None => return Err(Error::DelegationError(DelegationError::DelegationTagParse)),
            Some(a) => a,
        };
        if arr.len() != 4 {
            return Err(Error::DelegationError(DelegationError::DelegationTagParse));
        }
        if arr[0].as_str().unwrap_or("") != DELEGATION_KEYWORD {
            return Err(Error::DelegationError(DelegationError::DelegationTagParse));
        }
        let delegator_pubkey = PublicKey::try_from_hex_string(arr[1].as_str().unwrap_or(""))?;
        let conditions = Conditions::from_str(arr[2].as_str().unwrap_or(""))?;
        let signature = Signature::try_from_hex_string(arr[3].as_str().unwrap_or(""))?;
        Ok(DelegationTag {
            delegator_pubkey,
            conditions: conditions.to_string(),
            signature,
        })
    }
}

impl fmt::Display for DelegationTag {
    /// Return tag in JSON string format
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.as_json())
    }
}

impl FromStr for DelegationTag {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_json(s)
    }
}

/// A condition from the delegation conditions.
#[derive(Clone)]
pub(crate) enum Condition {
    /// Event kind, e.g. kind=1
    Kind(u64),
    /// Creation time before, e.g. created_at<1679000000
    CreatedBefore(u64),
    /// Creation time after, e.g. created_at>1676000000
    CreatedAfter(u64),
}

/// Set of conditions of a delegation.
#[derive(Clone)]
pub(crate) struct Conditions {
    cond: Vec<Condition>,
}

/// Represents properties of an event, relevant for delegation
#[derive(Debug, Clone, Copy)]
pub struct EventProperties {
    /// Event kind. For simplicity/flexibility, numeric type is used.
    kind: u64,
    /// Creation time, as unix timestamp
    created_time: u64,
}

impl Condition {
    /// Evaluate whether an event satisfies this condition
    pub(crate) fn evaluate(&self, ep: &EventProperties) -> Result<(), ValidationError> {
        match self {
            Self::Kind(k) => {
                if ep.kind != *k {
                    return Err(ValidationError::InvalidKind);
                }
            }
            Self::CreatedBefore(t) => {
                if ep.created_time >= *t {
                    return Err(ValidationError::CreatedTooLate);
                }
            }
            Self::CreatedAfter(t) => {
                if ep.created_time <= *t {
                    return Err(ValidationError::CreatedTooEarly);
                }
            }
        }
        Ok(())
    }
}

impl Copy for Condition {}

impl ToString for Condition {
    fn to_string(&self) -> String {
        match self {
            Self::Kind(k) => format!("kind={k}"),
            Self::CreatedBefore(t) => format!("created_at<{t}"),
            Self::CreatedAfter(t) => format!("created_at>{t}"),
        }
    }
}

impl FromStr for Condition {
    type Err = DelegationError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some(kind) = s.strip_prefix("kind=") {
            let n = u64::from_str(kind)?;
            return Ok(Self::Kind(n));
        }
        if let Some(created_before) = s.strip_prefix("created_at<") {
            let n = u64::from_str(created_before)?;
            return Ok(Self::CreatedBefore(n));
        }
        if let Some(created_after) = s.strip_prefix("created_at>") {
            let n = u64::from_str(created_after)?;
            return Ok(Self::CreatedAfter(n));
        }
        Err(DelegationError::ConditionsParseInvalidCondition)
    }
}

impl Conditions {
    pub(crate) fn new() -> Self {
        Self { cond: Vec::new() }
    }

    #[cfg(test)]
    pub(crate) fn add(&mut self, cond: Condition) {
        self.cond.push(cond);
    }

    /// Evaluate whether an event satisfies all these conditions
    fn evaluate(&self, ep: &EventProperties) -> Result<(), ValidationError> {
        for c in &self.cond {
            c.evaluate(ep)?;
        }
        Ok(())
    }
}

impl ToString for Conditions {
    fn to_string(&self) -> String {
        // convert parts, join
        self.cond
            .iter()
            .map(|c| c.to_string())
            .collect::<Vec<String>>()
            .join("&")
    }
}

impl FromStr for Conditions {
    type Err = DelegationError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Ok(Self::new());
        }
        let cond = s
            .split('&')
            .map(Condition::from_str)
            .collect::<Result<Vec<Condition>, Self::Err>>()?;
        Ok(Self { cond })
    }
}

impl EventProperties {
    /// Create new with values
    pub fn new(event_kind: u64, created_time: u64) -> Self {
        Self {
            kind: event_kind,
            created_time,
        }
    }

    /// Create from an Event
    pub fn from_event(event: &Event) -> Self {
        Self {
            kind: u64::from(event.kind),
            created_time: event.created_at.0 as u64,
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn test_create_delegation_tag() {
        let delegator_privkey = PrivateKey::try_from_bech32_string(
            "nsec1ktekw0hr5evjs0n9nyyquz4sue568snypy2rwk5mpv6hl2hq3vtsk0kpae",
        )
        .unwrap();
        let delegatee_pubkey = PublicKey::try_from_bech32_string(
            "npub1h652adkpv4lr8k66cadg8yg0wl5wcc29z4lyw66m3rrwskcl4v6qr82xez",
        )
        .unwrap();
        let conditions = "kind=1&created_at>1676067553&created_at<1678659553".to_string();

        let tag =
            DelegationTag::create(&delegator_privkey, &delegatee_pubkey, &conditions).unwrap();

        // verify signature (it's variable)
        let verify_result = verify_delegation_signature(
            &delegator_privkey.public_key(),
            &tag.get_signature(),
            &delegatee_pubkey,
            conditions,
        );
        assert!(verify_result.is_ok());

        // signature changes, cannot compare to expected constant, use signature from result
        let expected = format!(
            "[\"delegation\",\"1a459a8a6aa6441d480ba665fb8fb21a4cfe8bcacb7d87300f8046a558a3fce4\",\"kind=1&created_at>1676067553&created_at<1678659553\",\"{}\"]",
            &tag.signature.as_hex_string());
        assert_eq!(tag.to_string(), expected);
    }

    #[test]
    fn test_validate_delegation_tag() {
        let delegator_prikey = PrivateKey::try_from_bech32_string(
            "nsec1ktekw0hr5evjs0n9nyyquz4sue568snypy2rwk5mpv6hl2hq3vtsk0kpae",
        )
        .unwrap();
        let delegatee_pubkey = PublicKey::try_from_bech32_string(
            "npub1h652adkpv4lr8k66cadg8yg0wl5wcc29z4lyw66m3rrwskcl4v6qr82xez",
        )
        .unwrap();
        let conditions = "kind=1&created_at>1676067553&created_at<1678659553".to_string();

        let tag = DelegationTag::create(&delegator_prikey, &delegatee_pubkey, &conditions).unwrap();

        assert!(DelegationTag::validate(
            &tag,
            &delegatee_pubkey,
            &EventProperties::new(1, 1677000000)
        )
        .is_ok());
    }

    #[test]
    fn test_delegation_tag_parse_and_validate() {
        let tag_str = "[\"delegation\",\"1a459a8a6aa6441d480ba665fb8fb21a4cfe8bcacb7d87300f8046a558a3fce4\",\"kind=1&created_at>1676067553&created_at<1678659553\",\"369aed09c1ad52fceb77ecd6c16f2433eac4a3803fc41c58876a5b60f4f36b9493d5115e5ec5a0ce6c3668ffe5b58d47f2cbc97233833bb7e908f66dbbbd9d36\"]";
        let delegatee_pubkey = PublicKey::try_from_bech32_string(
            "npub1h652adkpv4lr8k66cadg8yg0wl5wcc29z4lyw66m3rrwskcl4v6qr82xez",
        )
        .unwrap();

        let tag = DelegationTag::from_str(tag_str).unwrap();

        assert!(DelegationTag::validate(
            &tag,
            &delegatee_pubkey,
            &EventProperties::new(1, 1677000000)
        )
        .is_ok());

        // additional test: verify a value from inside the tag
        assert_eq!(
            tag.get_conditions(),
            "kind=1&created_at>1676067553&created_at<1678659553"
        );

        // additional test: try validation with invalid values, invalid event kind
        match DelegationTag::validate(
            &tag,
            &delegatee_pubkey,
            &EventProperties::new(5, 1677000000),
        )
        .err()
        .unwrap()
        {
            Error::DelegationError(DelegationError::ConditionsValidation(e)) => {
                assert_eq!(e, ValidationError::InvalidKind)
            }
            _ => panic!("Expected ConditionsValidation"),
        };
    }

    #[test]
    fn test_sign_delegation_verify_delegation_signature() {
        let delegator_prikey = PrivateKey::try_from_hex_string(
            "ee35e8bb71131c02c1d7e73231daa48e9953d329a4b701f7133c8f46dd21139c",
        )
        .unwrap();
        let delegatee_pubkey = PublicKey::try_from_bech32_string(
            "npub1gae33na4gfaeelrx48arwc2sc8wmccs3tt38emmjg9ltjktfzwtqtl4l6u",
        )
        .unwrap();
        let conditions = "kind=1&created_at>1674834236&created_at<1677426236".to_string();

        let signature =
            sign_delegation(&delegator_prikey, &delegatee_pubkey, conditions.clone()).unwrap();

        // signature is changing, validate by verify method
        let verify_result = verify_delegation_signature(
            &delegator_prikey.public_key(),
            &signature,
            &delegatee_pubkey,
            conditions,
        );
        assert!(verify_result.is_ok());
    }

    #[test]
    fn test_sign_delegation_verify_lowlevel() {
        let delegator_prikey = PrivateKey::try_from_hex_string(
            "ee35e8bb71131c02c1d7e73231daa48e9953d329a4b701f7133c8f46dd21139c",
        )
        .unwrap();
        let delegatee_pubkey = PublicKey::try_from_bech32_string(
            "npub1gae33na4gfaeelrx48arwc2sc8wmccs3tt38emmjg9ltjktfzwtqtl4l6u",
        )
        .unwrap();
        let conditions = "kind=1&created_at>1674834236&created_at<1677426236";

        let signature =
            sign_delegation(&delegator_prikey, &delegatee_pubkey, conditions.to_string()).unwrap();

        // signature is changing, validate by lowlevel verify
        let unhashed_token: String = format!(
            "nostr:delegation:{}:{}",
            delegatee_pubkey.as_hex_string(),
            conditions
        );
        let verify_result = delegator_prikey
            .public_key()
            .verify(unhashed_token.as_bytes(), &signature);
        assert!(verify_result.is_ok());
    }

    #[test]
    fn test_verify_delegation_signature() {
        let delegator_prikey = PrivateKey::try_from_hex_string(
            "ee35e8bb71131c02c1d7e73231daa48e9953d329a4b701f7133c8f46dd21139c",
        )
        .unwrap();
        // use one concrete signature
        let signature = Signature::try_from_hex_string("f9f00fcf8480686d9da6dfde1187d4ba19c54f6ace4c73361a14db429c4b96eb30b29283d6ea1f06ba9e18e06e408244c689039ddadbacffc56060f3da5b04b8").unwrap();
        let delegatee_pubkey = PublicKey::try_from_bech32_string(
            "npub1gae33na4gfaeelrx48arwc2sc8wmccs3tt38emmjg9ltjktfzwtqtl4l6u",
        )
        .unwrap();
        let conditions = "kind=1&created_at>1674834236&created_at<1677426236".to_string();

        let verify_result = verify_delegation_signature(
            &delegator_prikey.public_key(),
            &signature,
            &delegatee_pubkey,
            conditions,
        );
        assert!(verify_result.is_ok());
    }

    #[test]
    fn test_delegation_token() {
        let delegatee_pubkey = PublicKey::try_from_bech32_string(
            "npub1gae33na4gfaeelrx48arwc2sc8wmccs3tt38emmjg9ltjktfzwtqtl4l6u",
        )
        .unwrap();
        let conditions = "kind=1&created_at>1674834236&created_at<1677426236";
        let unhashed_token: String = delegation_token(&delegatee_pubkey, &conditions);
        assert_eq!(
                unhashed_token,
                "nostr:delegation:477318cfb5427b9cfc66a9fa376150c1ddbc62115ae27cef72417eb959691396:kind=1&created_at>1674834236&created_at<1677426236"
            );
    }

    #[test]
    fn test_delegation_tag_to_json() {
        let delegator_privkey = PrivateKey::try_from_bech32_string(
            "nsec1ktekw0hr5evjs0n9nyyquz4sue568snypy2rwk5mpv6hl2hq3vtsk0kpae",
        )
        .unwrap();
        let conditions = "kind=1&created_at<1678659553".to_string();
        let signature = Signature::try_from_hex_string("435091ab4c4a11e594b1a05e0fa6c2f6e3b6eaa87c53f2981a3d6980858c40fdcaffde9a4c461f352a109402a4278ff4dbf90f9ebd05f96dac5ae36a6364a976").unwrap();
        let d = DelegationTag {
            delegator_pubkey: delegator_privkey.public_key(),
            conditions,
            signature,
        };
        let tag = d.as_json();
        assert_eq!(tag, "[\"delegation\",\"1a459a8a6aa6441d480ba665fb8fb21a4cfe8bcacb7d87300f8046a558a3fce4\",\"kind=1&created_at<1678659553\",\"435091ab4c4a11e594b1a05e0fa6c2f6e3b6eaa87c53f2981a3d6980858c40fdcaffde9a4c461f352a109402a4278ff4dbf90f9ebd05f96dac5ae36a6364a976\"]");
    }

    #[test]
    fn test_delegation_tag_from_str() {
        let tag_str = "[\"delegation\",\"1a459a8a6aa6441d480ba665fb8fb21a4cfe8bcacb7d87300f8046a558a3fce4\",\"kind=1&created_at>1676067553&created_at<1678659553\",\"369aed09c1ad52fceb77ecd6c16f2433eac4a3803fc41c58876a5b60f4f36b9493d5115e5ec5a0ce6c3668ffe5b58d47f2cbc97233833bb7e908f66dbbbd9d36\"]";

        let tag = DelegationTag::from_str(tag_str).unwrap();

        assert_eq!(tag.to_string(), tag_str);
        assert_eq!(
            tag.get_conditions(),
            "kind=1&created_at>1676067553&created_at<1678659553"
        );
        assert_eq!(
            tag.get_delegator_pubkey().try_as_bech32_string().unwrap(),
            "npub1rfze4zn25ezp6jqt5ejlhrajrfx0az72ed7cwvq0spr22k9rlnjq93lmd4"
        );
    }

    #[test]
    fn test_validate_delegation_tag_negative() {
        let delegator_prikey = PrivateKey::try_from_bech32_string(
            "nsec1ktekw0hr5evjs0n9nyyquz4sue568snypy2rwk5mpv6hl2hq3vtsk0kpae",
        )
        .unwrap();
        let delegatee_pubkey = PublicKey::try_from_bech32_string(
            "npub1h652adkpv4lr8k66cadg8yg0wl5wcc29z4lyw66m3rrwskcl4v6qr82xez",
        )
        .unwrap();
        let conditions = "kind=1&created_at>1676067553&created_at<1678659553".to_string();

        let tag = DelegationTag::create(&delegator_prikey, &delegatee_pubkey, &conditions).unwrap();

        // positive
        assert!(DelegationTag::validate(
            &tag,
            &delegatee_pubkey,
            &EventProperties::new(1, 1677000000)
        )
        .is_ok());

        // signature verification fails if wrong delegatee key is given
        let wrong_pubkey = PublicKey::try_from_bech32_string(
            "npub1zju3cgxq9p6f2c2jzrhhwuse94p7efkj5dp59eerh53hqd08j4dszevd7s",
        )
        .unwrap();
        // Note: Error cannot be tested simply  using equality
        match DelegationTag::validate(&tag, &wrong_pubkey, &EventProperties::new(1, 1677000000))
            .err()
            .unwrap()
        {
            Error::DelegationError(DelegationError::ConditionsValidation(e)) => {
                assert_eq!(e, ValidationError::InvalidSignature)
            }
            _ => panic!("Expected ConditionsValidation"),
        }

        // wrong event kind
        match DelegationTag::validate(
            &tag,
            &delegatee_pubkey,
            &EventProperties::new(9, 1677000000),
        )
        .err()
        .unwrap()
        {
            Error::DelegationError(DelegationError::ConditionsValidation(e)) => {
                assert_eq!(e, ValidationError::InvalidKind)
            }
            _ => panic!("Expected ConditionsValidation"),
        };

        // wrong creation time
        match DelegationTag::validate(
            &tag,
            &delegatee_pubkey,
            &EventProperties::new(1, 1679000000),
        )
        .err()
        .unwrap()
        {
            Error::DelegationError(DelegationError::ConditionsValidation(e)) => {
                assert_eq!(e, ValidationError::CreatedTooLate)
            }
            _ => panic!("Expected ConditionsValidation"),
        };
    }

    #[test]
    fn test_conditions_to_string() {
        let mut c = Conditions::new();
        c.add(Condition::Kind(1));
        assert_eq!(c.to_string(), "kind=1");
        c.add(Condition::CreatedAfter(1674834236));
        c.add(Condition::CreatedBefore(1677426236));
        assert_eq!(
            c.to_string(),
            "kind=1&created_at>1674834236&created_at<1677426236"
        );
    }

    #[test]
    fn test_conditions_parse() {
        let c = Conditions::from_str("kind=1&created_at>1674834236&created_at<1677426236").unwrap();
        assert_eq!(
            c.to_string(),
            "kind=1&created_at>1674834236&created_at<1677426236"
        );

        // special: empty string
        let c_empty = Conditions::from_str("").unwrap();
        assert_eq!(c_empty.to_string(), "");

        // one condition
        let c_one = Conditions::from_str("created_at<10000").unwrap();
        assert_eq!(c_one.to_string(), "created_at<10000");
    }

    #[test]
    fn test_conditions_parse_negative() {
        match Conditions::from_str("__invalid_condition__&kind=1")
            .err()
            .unwrap()
        {
            DelegationError::ConditionsParseInvalidCondition => {}
            _ => panic!("Exepected ConditionsParseInvalidCondition"),
        }
        match Conditions::from_str("kind=__invalid_number__")
            .err()
            .unwrap()
        {
            DelegationError::ConditionsParseNumeric(_) => {}
            _ => panic!("Exepected ConditionsParseNumeric"),
        }
    }

    #[test]
    fn test_conditions_evaluate() {
        let c_kind = Conditions::from_str("kind=3").unwrap();
        assert!(c_kind.evaluate(&EventProperties::new(3, 0)).is_ok());
        assert_eq!(
            c_kind.evaluate(&EventProperties::new(5, 0)).err().unwrap(),
            ValidationError::InvalidKind
        );

        let c_impossible = Conditions::from_str("kind=3&kind=4").unwrap();
        assert_eq!(
            c_impossible
                .evaluate(&EventProperties::new(3, 0))
                .err()
                .unwrap(),
            ValidationError::InvalidKind
        );

        let c_before = Conditions::from_str("created_at<1000").unwrap();
        assert!(c_before.evaluate(&EventProperties::new(3, 500)).is_ok());
        assert_eq!(
            c_before
                .evaluate(&EventProperties::new(3, 2000))
                .err()
                .unwrap(),
            ValidationError::CreatedTooLate
        );

        let c_after = Conditions::from_str("created_at>1000").unwrap();
        assert!(c_after.evaluate(&EventProperties::new(3, 2000)).is_ok());
        assert_eq!(
            c_after
                .evaluate(&EventProperties::new(3, 500))
                .err()
                .unwrap(),
            ValidationError::CreatedTooEarly
        );

        let c_complex =
            Conditions::from_str("kind=1&created_at>1676067553&created_at<1678659553").unwrap();
        assert!(c_complex
            .evaluate(&EventProperties::new(1, 1677000000))
            .is_ok());
        //assert_eq!(c_complex.evaluate(&EventProperties{ kind: 1, created_time: 1677000000}).err().unwrap(), ValidationError::InvalidKind);
        assert_eq!(
            c_complex
                .evaluate(&EventProperties::new(5, 1677000000))
                .err()
                .unwrap(),
            ValidationError::InvalidKind
        );
        assert_eq!(
            c_complex
                .evaluate(&EventProperties::new(1, 1674000000))
                .err()
                .unwrap(),
            ValidationError::CreatedTooEarly
        );
        assert_eq!(
            c_complex
                .evaluate(&EventProperties::new(1, 1699000000))
                .err()
                .unwrap(),
            ValidationError::CreatedTooLate
        );
    }
}

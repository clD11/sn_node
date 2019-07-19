// Copyright 2019 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

// TODO: make these tests work without mock too.
#![cfg(feature = "mock")]
#![forbid(
    exceeding_bitshifts,
    mutable_transmutes,
    no_mangle_const_items,
    unknown_crate_types,
    warnings
)]
#![deny(
    bad_style,
    deprecated,
    improper_ctypes,
    missing_docs,
    non_shorthand_field_patterns,
    overflowing_literals,
    plugin_as_library,
    stable_features,
    unconditional_recursion,
    unknown_lints,
    unsafe_code,
    unused,
    unused_allocation,
    unused_attributes,
    unused_comparisons,
    unused_features,
    unused_parens,
    while_true
)]
#![warn(
    trivial_casts,
    trivial_numeric_casts,
    unused_extern_crates,
    unused_import_braces,
    unused_qualifications,
    unused_results
)]
#![allow(
    box_pointers,
    missing_copy_implementations,
    missing_debug_implementations,
    variant_size_differences
)]

#[macro_use]
mod common;

use self::common::{Environment, TestClientTrait};
use maplit::btreemap;
use rand::{distributions::Standard, Rng};
use safe_nd::{
    AData, ADataAddress, ADataAppend, ADataEntry, ADataIndex, ADataOwner, ADataPubPermissionSet,
    ADataPubPermissions, ADataUnpubPermissionSet, ADataUnpubPermissions, ADataUser, AppPermissions,
    AppendOnlyData, Coins, EntryError, Error as NdError, IData, IDataAddress, LoginPacket, MData,
    MDataAction, MDataAddress, MDataPermissionSet, MDataSeqEntryActions, MDataUnseqEntryActions,
    MDataValue, PubImmutableData, PubSeqAppendOnlyData, PubUnseqAppendOnlyData, PublicKey, Request,
    Result as NdResult, SeqAppendOnly, SeqMutableData, Transaction, UnpubImmutableData,
    UnpubSeqAppendOnlyData, UnpubUnseqAppendOnlyData, UnseqAppendOnly, UnseqMutableData, XorName,
};
use safe_vault::COST_OF_PUT;
use std::collections::{BTreeMap, BTreeSet};
use unwrap::unwrap;

#[test]
fn client_connects() {
    let mut env = Environment::new();
    let client = env.new_connected_client();
    let _app = env.new_connected_app(client.public_id().clone());
}

////////////////////////////////////////////////////////////////////////////////
//
// Login packets
//
////////////////////////////////////////////////////////////////////////////////

#[test]
fn login_packets() {
    let mut env = Environment::new();
    let mut client = env.new_connected_client();

    let login_packet_data = vec![0; 32];
    let login_packet_locator: XorName = env.rng().gen();

    common::create_balance_from_nano(&mut env, &mut client, 0, None);

    // Try to get a login packet that does not exist yet.
    common::send_request_expect_err(
        &mut env,
        &mut client,
        Request::GetLoginPacket(login_packet_locator),
        NdError::NoSuchLoginPacket,
    );

    // Create a new login packet.
    let login_packet = unwrap!(LoginPacket::new(
        login_packet_locator,
        *client.public_id().public_key(),
        login_packet_data.clone(),
        client.sign(&login_packet_data),
    ));

    common::perform_mutation(
        &mut env,
        &mut client,
        Request::CreateLoginPacket(login_packet.clone()),
    );

    // Try to get the login packet data and signature.
    let (data, sig) = common::get_from_response(
        &mut env,
        &mut client,
        Request::GetLoginPacket(login_packet_locator),
    );
    assert_eq!(data, login_packet_data);
    unwrap!(client.public_id().public_key().verify(&sig, &data));

    // Putting login packet to the same address should fail.
    common::send_request_expect_err(
        &mut env,
        &mut client,
        Request::CreateLoginPacket(login_packet),
        NdError::LoginPacketExists,
    );

    // Getting login packet from non-owning client should fail.
    {
        let mut client = env.new_connected_client();
        common::send_request_expect_err(
            &mut env,
            &mut client,
            Request::GetLoginPacket(login_packet_locator),
            NdError::AccessDenied,
        );
    }
}

#[test]
fn update_login_packet() {
    let mut env = Environment::new();
    let mut client = env.new_connected_client();

    common::create_balance_from_nano(&mut env, &mut client, 0, None);

    let login_packet_data = vec![0; 32];
    let login_packet_locator: XorName = env.rng().gen();

    // Create a new login packet.
    let login_packet = unwrap!(LoginPacket::new(
        login_packet_locator,
        *client.public_id().public_key(),
        login_packet_data.clone(),
        client.sign(&login_packet_data),
    ));

    common::perform_mutation(
        &mut env,
        &mut client,
        Request::CreateLoginPacket(login_packet.clone()),
    );

    // Update the login packet data.
    let new_login_packet_data = vec![1; 32];
    let client_public_key = *client.public_id().public_key();
    let signature = client.sign(&new_login_packet_data);
    common::perform_mutation(
        &mut env,
        &mut client,
        Request::UpdateLoginPacket(unwrap!(LoginPacket::new(
            login_packet_locator,
            client_public_key,
            new_login_packet_data.clone(),
            signature,
        ))),
    );

    // Try to get the login packet data and signature.
    let (data, sig) = common::get_from_response(
        &mut env,
        &mut client,
        Request::GetLoginPacket(login_packet_locator),
    );
    assert_eq!(data, new_login_packet_data);
    unwrap!(client.public_id().public_key().verify(&sig, &data));

    // Updating login packet from non-owning client should fail.
    {
        let mut client = env.new_connected_client();
        common::send_request_expect_err(
            &mut env,
            &mut client,
            Request::UpdateLoginPacket(login_packet),
            NdError::AccessDenied,
        );
    }
}

////////////////////////////////////////////////////////////////////////////////
//
// Coins
//
////////////////////////////////////////////////////////////////////////////////

#[test]
fn coin_operations() {
    let mut env = Environment::new();

    let mut client_a = env.new_connected_client();
    let mut client_b = env.new_connected_client();

    common::send_request_expect_ok(
        &mut env,
        &mut client_a,
        Request::GetBalance,
        unwrap!(Coins::from_nano(0)),
    );

    // Create A's balance
    let public_key = *client_a.public_id().public_key();
    let amount = unwrap!(Coins::from_nano(10));
    let mut expected = Transaction { id: 0, amount };
    common::send_request_expect_ok(
        &mut env,
        &mut client_a,
        Request::CreateBalance {
            new_balance_owner: public_key,
            amount,
            transaction_id: 0,
        },
        expected,
    );

    common::send_request_expect_ok(&mut env, &mut client_a, Request::GetBalance, amount);

    // Create B's balance
    let mut amount_b = unwrap!(Coins::from_nano(1));
    expected.amount = amount_b;
    common::send_request_expect_ok(
        &mut env,
        &mut client_a,
        Request::CreateBalance {
            new_balance_owner: *client_b.public_id().public_key(),
            amount: amount_b,
            transaction_id: 0,
        },
        expected,
    );

    let mut amount_a = unwrap!(Coins::from_nano(9));
    common::send_request_expect_ok(&mut env, &mut client_a, Request::GetBalance, amount_a);
    common::send_request_expect_ok(&mut env, &mut client_b, Request::GetBalance, amount_b);

    // Transfer coins from A to B
    expected.id = 1;
    expected.amount = unwrap!(Coins::from_nano(2));
    common::send_request_expect_ok(
        &mut env,
        &mut client_a,
        Request::TransferCoins {
            destination: *client_b.public_id().name(),
            amount: unwrap!(Coins::from_nano(2)),
            transaction_id: 1,
        },
        expected,
    );

    amount_a = unwrap!(Coins::from_nano(7));
    amount_b = unwrap!(Coins::from_nano(3));
    common::send_request_expect_ok(&mut env, &mut client_a, Request::GetBalance, amount_a);
    common::send_request_expect_ok(&mut env, &mut client_b, Request::GetBalance, amount_b);
}

////////////////////////////////////////////////////////////////////////////////
//
// Append-only data
//
////////////////////////////////////////////////////////////////////////////////

#[test]
fn put_append_only_data() {
    let mut env = Environment::new();
    let mut client_a = env.new_connected_client();
    let mut client_b = env.new_connected_client();

    let owner_a = ADataOwner {
        public_key: *client_a.public_id().public_key(),
        entries_index: 0,
        permissions_index: 0,
    };

    // Published sequential data
    let pub_seq_adata_name: XorName = env.rng().gen();
    let mut pub_seq_adata = PubSeqAppendOnlyData::new(pub_seq_adata_name, 100);
    unwrap!(pub_seq_adata.append_owner(owner_a, 0));
    unwrap!(pub_seq_adata.append(
        vec![ADataEntry {
            key: b"one".to_vec(),
            value: b"pub sec".to_vec()
        }],
        0
    ));
    unwrap!(pub_seq_adata.append(
        vec![ADataEntry {
            key: b"two".to_vec(),
            value: b"pub sec".to_vec()
        }],
        1
    ));
    let pub_seq_adata = AData::PubSeq(pub_seq_adata);

    // Published unsequential data
    let pub_unseq_adata_name: XorName = env.rng().gen();
    let mut pub_unseq_adata = PubUnseqAppendOnlyData::new(pub_unseq_adata_name, 100);
    unwrap!(pub_unseq_adata.append_owner(owner_a, 0));
    unwrap!(pub_unseq_adata.append(vec![ADataEntry {
        key: b"one".to_vec(),
        value: b"pub unsec".to_vec()
    }]));
    unwrap!(pub_unseq_adata.append(vec![ADataEntry {
        key: b"two".to_vec(),
        value: b"pub unsec".to_vec()
    }]));
    let pub_unseq_adata = AData::PubUnseq(pub_unseq_adata);

    // Unpublished sequential
    let unpub_seq_adata_name: XorName = env.rng().gen();
    let mut unpub_seq_adata = UnpubSeqAppendOnlyData::new(unpub_seq_adata_name, 100);
    unwrap!(unpub_seq_adata.append_owner(owner_a, 0));
    unwrap!(unpub_seq_adata.append(
        vec![ADataEntry {
            key: b"one".to_vec(),
            value: b"unpub sec".to_vec()
        }],
        0
    ));
    unwrap!(unpub_seq_adata.append(
        vec![ADataEntry {
            key: b"two".to_vec(),
            value: b"unpub sec".to_vec()
        }],
        1
    ));
    let unpub_seq_adata = AData::UnpubSeq(unpub_seq_adata);

    // Unpublished unsequential data
    let unpub_unseq_adata_name: XorName = env.rng().gen();
    let mut unpub_unseq_adata = UnpubUnseqAppendOnlyData::new(unpub_unseq_adata_name, 100);
    unwrap!(unpub_unseq_adata.append_owner(owner_a, 0));
    unwrap!(unpub_unseq_adata.append(vec![ADataEntry {
        key: b"one".to_vec(),
        value: b"unpub unsec".to_vec()
    }]));
    unwrap!(unpub_unseq_adata.append(vec![ADataEntry {
        key: b"two".to_vec(),
        value: b"unpub unsec".to_vec()
    }]));
    let unpub_unseq_adata = AData::UnpubUnseq(unpub_unseq_adata);

    // TODO - Enable this once we're passed phase 1
    // First try to put some data without any associated balance.
    if false {
        common::send_request_expect_err(
            &mut env,
            &mut client_a,
            Request::PutAData(pub_seq_adata.clone()),
            NdError::AccessDenied,
        );
        common::send_request_expect_err(
            &mut env,
            &mut client_a,
            Request::PutAData(pub_unseq_adata.clone()),
            NdError::AccessDenied,
        );
        common::send_request_expect_err(
            &mut env,
            &mut client_a,
            Request::PutAData(unpub_seq_adata.clone()),
            NdError::AccessDenied,
        );
        common::send_request_expect_err(
            &mut env,
            &mut client_a,
            Request::PutAData(unpub_unseq_adata.clone()),
            NdError::AccessDenied,
        );
    }

    let start_nano = 1_000_000_000_000;
    common::create_balance_from_nano(&mut env, &mut client_a, start_nano, None);

    // Check that client B cannot put A's data
    common::send_request_expect_err(
        &mut env,
        &mut client_b,
        Request::PutAData(pub_seq_adata.clone()),
        NdError::InvalidOwners,
    );
    common::send_request_expect_err(
        &mut env,
        &mut client_b,
        Request::PutAData(pub_unseq_adata.clone()),
        NdError::InvalidOwners,
    );
    common::send_request_expect_err(
        &mut env,
        &mut client_b,
        Request::PutAData(unpub_seq_adata.clone()),
        NdError::InvalidOwners,
    );
    common::send_request_expect_err(
        &mut env,
        &mut client_b,
        Request::PutAData(unpub_unseq_adata.clone()),
        NdError::InvalidOwners,
    );

    // Put, this time with a balance and the correct owner
    common::perform_mutation(
        &mut env,
        &mut client_a,
        Request::PutAData(pub_seq_adata.clone()),
    );
    common::perform_mutation(
        &mut env,
        &mut client_a,
        Request::PutAData(pub_unseq_adata.clone()),
    );
    common::perform_mutation(
        &mut env,
        &mut client_a,
        Request::PutAData(unpub_seq_adata.clone()),
    );
    common::perform_mutation(
        &mut env,
        &mut client_a,
        Request::PutAData(unpub_unseq_adata.clone()),
    );

    // Get the data to verify
    common::send_request_expect_ok(
        &mut env,
        &mut client_a,
        Request::GetAData(*pub_seq_adata.address()),
        pub_seq_adata.clone(),
    );
    common::send_request_expect_ok(
        &mut env,
        &mut client_a,
        Request::GetAData(*pub_unseq_adata.address()),
        pub_unseq_adata.clone(),
    );
    common::send_request_expect_ok(
        &mut env,
        &mut client_a,
        Request::GetAData(*unpub_seq_adata.address()),
        unpub_seq_adata.clone(),
    );
    common::send_request_expect_ok(
        &mut env,
        &mut client_a,
        Request::GetAData(*unpub_unseq_adata.address()),
        unpub_unseq_adata.clone(),
    );

    // Verify that B cannot delete A's data
    common::send_request_expect_err(
        &mut env,
        &mut client_b,
        Request::DeleteAData(*pub_seq_adata.address()),
        NdError::InvalidOperation,
    );
    common::send_request_expect_err(
        &mut env,
        &mut client_b,
        Request::DeleteAData(*pub_unseq_adata.address()),
        NdError::InvalidOperation,
    );
    common::send_request_expect_err(
        &mut env,
        &mut client_b,
        Request::DeleteAData(*unpub_seq_adata.address()),
        NdError::AccessDenied,
    );
    common::send_request_expect_err(
        &mut env,
        &mut client_b,
        Request::DeleteAData(*unpub_unseq_adata.address()),
        NdError::AccessDenied,
    );

    // Delete the data
    common::send_request_expect_err(
        &mut env,
        &mut client_a,
        Request::DeleteAData(*pub_seq_adata.address()),
        NdError::InvalidOperation,
    );
    common::send_request_expect_err(
        &mut env,
        &mut client_a,
        Request::DeleteAData(*pub_unseq_adata.address()),
        NdError::InvalidOperation,
    );
    common::perform_mutation(
        &mut env,
        &mut client_a,
        Request::DeleteAData(*unpub_seq_adata.address()),
    );
    common::perform_mutation(
        &mut env,
        &mut client_a,
        Request::DeleteAData(*unpub_unseq_adata.address()),
    );

    // Delete again to test if it's gone
    common::send_request_expect_err(
        &mut env,
        &mut client_a,
        Request::DeleteAData(*unpub_seq_adata.address()),
        NdError::NoSuchData,
    );
    common::send_request_expect_err(
        &mut env,
        &mut client_a,
        Request::DeleteAData(*unpub_unseq_adata.address()),
        NdError::NoSuchData,
    );
}

#[test]
fn append_only_data_delete_data_doesnt_exist() {
    let mut env = Environment::new();
    let mut client = env.new_connected_client();

    let name: XorName = env.rng().gen();
    let tag = 100;

    common::send_request_expect_err(
        &mut env,
        &mut client,
        Request::DeleteAData(*AData::PubSeq(PubSeqAppendOnlyData::new(name, tag)).address()),
        NdError::InvalidOperation,
    );
    common::send_request_expect_err(
        &mut env,
        &mut client,
        Request::DeleteAData(*AData::PubUnseq(PubUnseqAppendOnlyData::new(name, tag)).address()),
        NdError::InvalidOperation,
    );
    common::send_request_expect_err(
        &mut env,
        &mut client,
        Request::DeleteAData(*AData::UnpubSeq(UnpubSeqAppendOnlyData::new(name, tag)).address()),
        NdError::AccessDenied,
    );
    common::send_request_expect_err(
        &mut env,
        &mut client,
        Request::DeleteAData(
            *AData::UnpubUnseq(UnpubUnseqAppendOnlyData::new(name, tag)).address(),
        ),
        NdError::AccessDenied,
    );

    let start_nano = 1_000_000_000_000;
    common::create_balance_from_nano(&mut env, &mut client, start_nano, None);

    common::send_request_expect_err(
        &mut env,
        &mut client,
        Request::DeleteAData(*AData::PubSeq(PubSeqAppendOnlyData::new(name, tag)).address()),
        NdError::InvalidOperation,
    );
    common::send_request_expect_err(
        &mut env,
        &mut client,
        Request::DeleteAData(*AData::PubUnseq(PubUnseqAppendOnlyData::new(name, tag)).address()),
        NdError::InvalidOperation,
    );
    common::send_request_expect_err(
        &mut env,
        &mut client,
        Request::DeleteAData(*AData::UnpubSeq(UnpubSeqAppendOnlyData::new(name, tag)).address()),
        NdError::NoSuchData,
    );
    common::send_request_expect_err(
        &mut env,
        &mut client,
        Request::DeleteAData(
            *AData::UnpubUnseq(UnpubUnseqAppendOnlyData::new(name, tag)).address(),
        ),
        NdError::NoSuchData,
    );
}

#[test]
fn get_pub_append_only_data() {
    let mut env = Environment::new();
    let mut client = env.new_connected_client();
    common::create_balance_from_nano(&mut env, &mut client, 0, None);

    let mut data = PubSeqAppendOnlyData::new(env.rng().gen(), 100);

    let owner = ADataOwner {
        public_key: *client.public_id().public_key(),
        entries_index: 0,
        permissions_index: 0,
    };
    unwrap!(data.append_owner(owner, 0));

    let data = AData::PubSeq(data);
    let address = *data.address();
    common::perform_mutation(&mut env, &mut client, Request::PutAData(data.clone()));

    // Success
    common::send_request_expect_ok(
        &mut env,
        &mut client,
        Request::GetAData(address),
        data.clone(),
    );

    // Failure - non-existing data
    let invalid_name: XorName = env.rng().gen();
    let invalid_address = ADataAddress::PubSeq {
        name: invalid_name,
        tag: 100,
    };

    common::send_request_expect_err(
        &mut env,
        &mut client,
        Request::GetAData(invalid_address),
        NdError::NoSuchData,
    );

    // Published data is gettable by non-owners too
    let mut other_client = env.new_connected_client();
    common::send_request_expect_ok(
        &mut env,
        &mut other_client,
        Request::GetAData(address),
        data,
    );
}

#[test]
fn get_unpub_append_only_data() {
    let mut env = Environment::new();
    let mut client = env.new_connected_client();

    common::create_balance_from_nano(&mut env, &mut client, 0, None);

    let mut data = UnpubSeqAppendOnlyData::new(env.rng().gen(), 100);

    let owner = ADataOwner {
        public_key: *client.public_id().public_key(),
        entries_index: 0,
        permissions_index: 0,
    };
    unwrap!(data.append_owner(owner, 0));

    let data = AData::UnpubSeq(data);
    let address = *data.address();
    common::perform_mutation(&mut env, &mut client, Request::PutAData(data.clone()));

    // Success
    common::send_request_expect_ok(&mut env, &mut client, Request::GetAData(address), data);

    // Failure - non-existing data
    let invalid_name: XorName = env.rng().gen();
    let invalid_address = ADataAddress::UnpubSeq {
        name: invalid_name,
        tag: 100,
    };

    common::send_request_expect_err(
        &mut env,
        &mut client,
        Request::GetAData(invalid_address),
        NdError::NoSuchData,
    );

    // Failure - get by non-owner not allowed
    let mut other_client = env.new_connected_client();
    common::create_balance_from_nano(&mut env, &mut other_client, 0, None);

    common::send_request_expect_err(
        &mut env,
        &mut other_client,
        Request::GetAData(address),
        NdError::InvalidPermissions,
    );
}

#[test]
fn append_only_data_get_entries() {
    let mut env = Environment::new();
    let mut client = env.new_connected_client();

    common::create_balance_from_nano(&mut env, &mut client, 0, None);

    let mut data = PubSeqAppendOnlyData::new(env.rng().gen(), 100);

    let owner = ADataOwner {
        public_key: *client.public_id().public_key(),
        entries_index: 0,
        permissions_index: 0,
    };

    unwrap!(data.append_owner(owner, 0));
    unwrap!(data.append(
        vec![
            ADataEntry::new(b"one".to_vec(), b"foo".to_vec()),
            ADataEntry::new(b"two".to_vec(), b"bar".to_vec()),
        ],
        0,
    ));

    let data = AData::PubSeq(data);
    let address = *data.address();
    common::perform_mutation(&mut env, &mut client, Request::PutAData(data.clone()));

    // GetADataRange
    let mut range_scenario = |start, end, expected_result| {
        common::send_request_expect_ok(
            &mut env,
            &mut client,
            Request::GetADataRange {
                address,
                range: (start, end),
            },
            expected_result,
        )
    };

    range_scenario(ADataIndex::FromStart(0), ADataIndex::FromStart(0), vec![]);
    range_scenario(
        ADataIndex::FromStart(0),
        ADataIndex::FromStart(1),
        vec![ADataEntry::new(b"one".to_vec(), b"foo".to_vec())],
    );
    range_scenario(
        ADataIndex::FromStart(1),
        ADataIndex::FromStart(2),
        vec![ADataEntry::new(b"two".to_vec(), b"bar".to_vec())],
    );
    range_scenario(
        ADataIndex::FromEnd(1),
        ADataIndex::FromEnd(0),
        vec![ADataEntry::new(b"two".to_vec(), b"bar".to_vec())],
    );
    range_scenario(
        ADataIndex::FromStart(0),
        ADataIndex::FromEnd(0),
        vec![
            ADataEntry::new(b"one".to_vec(), b"foo".to_vec()),
            ADataEntry::new(b"two".to_vec(), b"bar".to_vec()),
        ],
    );
    common::send_request_expect_err(
        &mut env,
        &mut client,
        Request::GetADataRange {
            address,
            range: (ADataIndex::FromStart(0), ADataIndex::FromStart(3)),
        },
        NdError::NoSuchEntry,
    );

    // GetADataLastEntry
    common::send_request_expect_ok(
        &mut env,
        &mut client,
        Request::GetADataLastEntry(address),
        ADataEntry::new(b"two".to_vec(), b"bar".to_vec()),
    );

    // GetADataValue
    common::send_request_expect_ok(
        &mut env,
        &mut client,
        Request::GetADataValue {
            address,
            key: b"one".to_vec(),
        },
        b"foo".to_vec(),
    );
}

#[test]
fn append_only_data_get_owners() {
    let mut env = Environment::new();
    let mut client = env.new_connected_client();
    common::create_balance_from_nano(&mut env, &mut client, 0, None);

    let name: XorName = env.rng().gen();
    let tag = 100;
    let mut data = PubSeqAppendOnlyData::new(name, tag);

    let owner_0 = ADataOwner {
        public_key: common::gen_public_key(env.rng()),
        entries_index: 0,
        permissions_index: 0,
    };
    let owner_1 = ADataOwner {
        public_key: common::gen_public_key(env.rng()),
        entries_index: 0,
        permissions_index: 0,
    };
    let owner_2 = ADataOwner {
        public_key: *client.public_id().public_key(),
        entries_index: 1,
        permissions_index: 0,
    };

    unwrap!(data.append_owner(owner_0, 0));
    unwrap!(data.append_owner(owner_1, 1));

    unwrap!(data.append(vec![ADataEntry::new(b"one".to_vec(), b"foo".to_vec())], 0));
    unwrap!(data.append_owner(owner_2, 2));

    let address = *data.address();
    common::perform_mutation(&mut env, &mut client, Request::PutAData(data.into()));

    let mut scenario = |owners_index, expected_response| {
        let req = Request::GetADataOwners {
            address,
            owners_index,
        };
        match expected_response {
            Ok(expected) => common::send_request_expect_ok(&mut env, &mut client, req, expected),
            Err(expected) => common::send_request_expect_err(&mut env, &mut client, req, expected),
        }
    };

    scenario(ADataIndex::FromStart(0), Ok(owner_0));
    scenario(ADataIndex::FromStart(1), Ok(owner_1));
    scenario(ADataIndex::FromStart(2), Ok(owner_2));
    scenario(ADataIndex::FromStart(3), Err(NdError::InvalidOwners));

    scenario(ADataIndex::FromEnd(0), Err(NdError::InvalidOwners));
    scenario(ADataIndex::FromEnd(1), Ok(owner_2));
    scenario(ADataIndex::FromEnd(2), Ok(owner_1));
    scenario(ADataIndex::FromEnd(3), Ok(owner_0));
    scenario(ADataIndex::FromEnd(4), Err(NdError::InvalidOwners));
}

#[test]
fn pub_append_only_data_get_permissions() {
    let mut env = Environment::new();
    let mut client = env.new_connected_client();
    common::create_balance_from_nano(&mut env, &mut client, 0, None);

    let name: XorName = env.rng().gen();
    let tag = 100;
    let mut data = PubSeqAppendOnlyData::new(name, tag);

    let owner = ADataOwner {
        public_key: *client.public_id().public_key(),
        entries_index: 0,
        permissions_index: 0,
    };

    unwrap!(data.append_owner(owner, 0));

    let perms_0 = ADataPubPermissions {
        permissions: btreemap![ADataUser::Anyone => ADataPubPermissionSet::new(true, false)],
        entries_index: 0,
        owners_index: 1,
    };
    unwrap!(data.append_permissions(perms_0.clone(), 0));

    let public_key = common::gen_public_key(env.rng());
    let perms_1 = ADataPubPermissions {
        permissions: btreemap![
            ADataUser::Anyone => ADataPubPermissionSet::new(false, false),
            ADataUser::Key(public_key) => ADataPubPermissionSet::new(true, false)
        ],
        entries_index: 0,
        owners_index: 1,
    };
    unwrap!(data.append_permissions(perms_1.clone(), 1));

    let address = *data.address();
    common::perform_mutation(&mut env, &mut client, Request::PutAData(data.into()));

    // GetPubADataUserPermissions
    let mut scenario = |permissions_index, user, expected_response| {
        let req = Request::GetPubADataUserPermissions {
            address,
            permissions_index,
            user,
        };
        match expected_response {
            Ok(expected) => common::send_request_expect_ok(&mut env, &mut client, req, expected),
            Err(expected) => common::send_request_expect_err(&mut env, &mut client, req, expected),
        }
    };

    scenario(
        ADataIndex::FromStart(0),
        ADataUser::Anyone,
        Ok(ADataPubPermissionSet::new(true, false)),
    );
    scenario(
        ADataIndex::FromStart(0),
        ADataUser::Key(public_key),
        Err(NdError::NoSuchEntry),
    );
    scenario(
        ADataIndex::FromStart(1),
        ADataUser::Anyone,
        Ok(ADataPubPermissionSet::new(false, false)),
    );
    scenario(
        ADataIndex::FromStart(1),
        ADataUser::Key(public_key),
        Ok(ADataPubPermissionSet::new(true, false)),
    );
    scenario(
        ADataIndex::FromStart(2),
        ADataUser::Anyone,
        Err(NdError::NoSuchEntry),
    );

    scenario(
        ADataIndex::FromEnd(1),
        ADataUser::Anyone,
        Ok(ADataPubPermissionSet::new(false, false)),
    );
    scenario(
        ADataIndex::FromEnd(2),
        ADataUser::Anyone,
        Ok(ADataPubPermissionSet::new(true, false)),
    );
    scenario(
        ADataIndex::FromEnd(3),
        ADataUser::Anyone,
        Err(NdError::NoSuchEntry),
    );

    // GetUnpubADataUserPermissions (failure - incorrect data kind)
    common::send_request_expect_err(
        &mut env,
        &mut client,
        Request::GetUnpubADataUserPermissions {
            address,
            permissions_index: ADataIndex::FromStart(1),
            public_key,
        },
        NdError::NoSuchData,
    );

    // GetADataPermissions
    let mut scenario = |permissions_index, expected_response| {
        let req = Request::GetADataPermissions {
            address,
            permissions_index,
        };
        match expected_response {
            Ok(expected) => common::send_request_expect_ok(&mut env, &mut client, req, expected),
            Err(expected) => common::send_request_expect_err(&mut env, &mut client, req, expected),
        }
    };

    scenario(ADataIndex::FromStart(0), Ok(perms_0));
    scenario(ADataIndex::FromStart(1), Ok(perms_1));
    scenario(ADataIndex::FromStart(2), Err(NdError::NoSuchEntry));
}

#[test]
fn unpub_append_only_data_get_permissions() {
    let mut env = Environment::new();
    let mut client = env.new_connected_client();

    let start_nano = 1_000_000_000_000;
    common::create_balance_from_nano(&mut env, &mut client, start_nano, None);

    let name: XorName = env.rng().gen();
    let tag = 100;
    let mut data = UnpubSeqAppendOnlyData::new(name, tag);

    let owner = ADataOwner {
        public_key: *client.public_id().public_key(),
        entries_index: 0,
        permissions_index: 0,
    };

    unwrap!(data.append_owner(owner, 0));

    let public_key_0 = common::gen_public_key(env.rng());
    let public_key_1 = common::gen_public_key(env.rng());

    let perms_0 = ADataUnpubPermissions {
        permissions: btreemap![
            public_key_0 => ADataUnpubPermissionSet::new(true, true, false)
        ],
        entries_index: 0,
        owners_index: 1,
    };
    unwrap!(data.append_permissions(perms_0.clone(), 0));

    let perms_1 = ADataUnpubPermissions {
        permissions: btreemap![
            public_key_0 => ADataUnpubPermissionSet::new(true, false, false),
            public_key_1 => ADataUnpubPermissionSet::new(true, true, true)
        ],
        entries_index: 0,
        owners_index: 1,
    };
    unwrap!(data.append_permissions(perms_1.clone(), 1));

    let address = *data.address();
    common::perform_mutation(&mut env, &mut client, Request::PutAData(data.into()));

    // GetUnpubADataUserPermissions
    let mut scenario = |permissions_index, public_key, expected_response| {
        let req = Request::GetUnpubADataUserPermissions {
            address,
            permissions_index,
            public_key,
        };
        match expected_response {
            Ok(expected) => common::send_request_expect_ok(&mut env, &mut client, req, expected),
            Err(expected) => common::send_request_expect_err(&mut env, &mut client, req, expected),
        }
    };

    scenario(
        ADataIndex::FromStart(0),
        public_key_0,
        Ok(ADataUnpubPermissionSet::new(true, true, false)),
    );
    scenario(
        ADataIndex::FromStart(0),
        public_key_1,
        Err(NdError::NoSuchEntry),
    );
    scenario(
        ADataIndex::FromStart(1),
        public_key_0,
        Ok(ADataUnpubPermissionSet::new(true, false, false)),
    );
    scenario(
        ADataIndex::FromStart(1),
        public_key_1,
        Ok(ADataUnpubPermissionSet::new(true, true, true)),
    );
    scenario(
        ADataIndex::FromStart(2),
        public_key_0,
        Err(NdError::NoSuchEntry),
    );

    scenario(
        ADataIndex::FromEnd(1),
        public_key_0,
        Ok(ADataUnpubPermissionSet::new(true, false, false)),
    );
    scenario(
        ADataIndex::FromEnd(2),
        public_key_0,
        Ok(ADataUnpubPermissionSet::new(true, true, false)),
    );
    scenario(
        ADataIndex::FromEnd(3),
        public_key_0,
        Err(NdError::NoSuchEntry),
    );

    // GetPubADataUserPermissions (failure - incorrect data kind)
    common::send_request_expect_err(
        &mut env,
        &mut client,
        Request::GetPubADataUserPermissions {
            address,
            permissions_index: ADataIndex::FromStart(1),
            user: ADataUser::Key(public_key_0),
        },
        NdError::NoSuchData,
    );

    // GetADataPermissions
    let mut scenario = |permissions_index, expected_response| {
        let req = Request::GetADataPermissions {
            address,
            permissions_index,
        };
        match expected_response {
            Ok(expected) => common::send_request_expect_ok(&mut env, &mut client, req, expected),
            Err(expected) => common::send_request_expect_err(&mut env, &mut client, req, expected),
        }
    };

    scenario(ADataIndex::FromStart(0), Ok(perms_0));
    scenario(ADataIndex::FromStart(1), Ok(perms_1));
    scenario(ADataIndex::FromStart(2), Err(NdError::NoSuchEntry));
}

#[test]
fn pub_append_only_data_put_permissions() {
    let mut env = Environment::new();
    let mut client_a = env.new_connected_client();
    let mut client_b = env.new_connected_client();

    let public_key_a = *client_a.public_id().public_key();
    let public_key_b = *client_b.public_id().public_key();

    let start_nano = 1_000_000_000_000;
    common::create_balance_from_nano(&mut env, &mut client_a, start_nano, None);
    common::create_balance_from_nano(&mut env, &mut client_b, start_nano, None);

    let name: XorName = env.rng().gen();
    let tag = 100;
    let mut data = PubSeqAppendOnlyData::new(name, tag);

    let owner = ADataOwner {
        public_key: *client_a.public_id().public_key(),
        entries_index: 0,
        permissions_index: 0,
    };

    unwrap!(data.append_owner(owner, 0));

    // Client A can manage permissions, but not B
    let perms_0 = ADataPubPermissions {
        permissions: btreemap![ADataUser::Key(public_key_a) => ADataPubPermissionSet::new(true, true)],
        entries_index: 0,
        owners_index: 1,
    };
    unwrap!(data.append_permissions(perms_0.clone(), 0));

    common::perform_mutation(
        &mut env,
        &mut client_a,
        Request::PutAData(AData::PubSeq(data.clone())),
    );

    // Before
    common::send_request_expect_ok(
        &mut env,
        &mut client_a,
        Request::GetADataPermissions {
            address: *data.address(),
            permissions_index: ADataIndex::FromStart(0),
        },
        perms_0,
    );
    common::send_request_expect_err(
        &mut env,
        &mut client_a,
        Request::GetADataPermissions {
            address: *data.address(),
            permissions_index: ADataIndex::FromStart(1),
        },
        NdError::NoSuchEntry,
    );

    let perms_1 = ADataPubPermissions {
        permissions: btreemap![
            ADataUser::Key(public_key_b) => ADataPubPermissionSet::new(true, true)
        ],
        entries_index: 0,
        owners_index: 1,
    };

    // Only client A has permissions to add permissions
    common::send_request_expect_err(
        &mut env,
        &mut client_b,
        Request::AddPubADataPermissions {
            address: *data.address(),
            permissions: perms_1.clone(),
            permissions_idx: 1,
        },
        // TODO: InvalidPermissions because client B doesn't have any key avail. We should consider
        // changing this behaviour to AccessDenied.
        NdError::InvalidPermissions,
    );

    common::perform_mutation(
        &mut env,
        &mut client_a,
        Request::AddPubADataPermissions {
            address: *data.address(),
            permissions: perms_1.clone(),
            permissions_idx: 1,
        },
    );

    // Check that the permissions have been updated
    common::send_request_expect_ok(
        &mut env,
        &mut client_a,
        Request::GetADataPermissions {
            address: *data.address(),
            permissions_index: ADataIndex::FromStart(1),
        },
        perms_1,
    );
}

#[test]
fn unpub_append_only_data_put_permissions() {
    let mut env = Environment::new();
    let mut client_a = env.new_connected_client();
    let mut client_b = env.new_connected_client();

    let public_key_a = *client_a.public_id().public_key();
    let public_key_b = *client_b.public_id().public_key();

    let start_nano = 1_000_000_000_000;
    common::create_balance_from_nano(&mut env, &mut client_a, start_nano, None);
    common::create_balance_from_nano(&mut env, &mut client_b, start_nano, None);

    let name: XorName = env.rng().gen();
    let tag = 100;
    let mut data = UnpubSeqAppendOnlyData::new(name, tag);

    let owner = ADataOwner {
        public_key: *client_a.public_id().public_key(),
        entries_index: 0,
        permissions_index: 0,
    };

    unwrap!(data.append_owner(owner, 0));

    // Client A can manage permissions, but not B
    let perms_0 = ADataUnpubPermissions {
        permissions: btreemap![public_key_a => ADataUnpubPermissionSet::new(true, true, true)],
        entries_index: 0,
        owners_index: 1,
    };
    unwrap!(data.append_permissions(perms_0.clone(), 0));

    common::perform_mutation(
        &mut env,
        &mut client_a,
        Request::PutAData(AData::UnpubSeq(data.clone())),
    );

    // Before
    common::send_request_expect_ok(
        &mut env,
        &mut client_a,
        Request::GetADataPermissions {
            address: *data.address(),
            permissions_index: ADataIndex::FromStart(0),
        },
        perms_0,
    );
    common::send_request_expect_err(
        &mut env,
        &mut client_a,
        Request::GetADataPermissions {
            address: *data.address(),
            permissions_index: ADataIndex::FromStart(1),
        },
        NdError::NoSuchEntry,
    );

    let perms_1 = ADataUnpubPermissions {
        permissions: btreemap![
            public_key_b => ADataUnpubPermissionSet::new(true, true, true)
        ],
        entries_index: 0,
        owners_index: 1,
    };

    // Only client A has permissions to add permissions
    common::send_request_expect_err(
        &mut env,
        &mut client_b,
        Request::AddUnpubADataPermissions {
            address: *data.address(),
            permissions: perms_1.clone(),
            permissions_idx: 1,
        },
        // TODO: InvalidPermissions because client B doesn't have any key avail. We should consider
        // changing this behaviour to AccessDenied.
        NdError::InvalidPermissions,
    );

    common::perform_mutation(
        &mut env,
        &mut client_a,
        Request::AddUnpubADataPermissions {
            address: *data.address(),
            permissions: perms_1.clone(),
            permissions_idx: 1,
        },
    );

    // Check that the permissions have been updated
    common::send_request_expect_ok(
        &mut env,
        &mut client_a,
        Request::GetADataPermissions {
            address: *data.address(),
            permissions_index: ADataIndex::FromStart(1),
        },
        perms_1,
    );
}

#[test]
fn append_only_data_put_owners() {
    let mut env = Environment::new();
    let mut client_a = env.new_connected_client();
    let mut client_b = env.new_connected_client();

    let public_key_a = *client_a.public_id().public_key();
    let public_key_b = *client_b.public_id().public_key();

    let start_nano = 1_000_000_000_000;
    common::create_balance_from_nano(&mut env, &mut client_a, start_nano, None);
    common::create_balance_from_nano(&mut env, &mut client_b, start_nano, None);

    let name: XorName = env.rng().gen();
    let tag = 100;
    let mut data = PubSeqAppendOnlyData::new(name, tag);

    let owner_0 = ADataOwner {
        public_key: public_key_a,
        entries_index: 0,
        permissions_index: 0,
    };
    unwrap!(data.append_owner(owner_0, 0));

    let perms_0 = ADataPubPermissions {
        permissions: btreemap![ADataUser::Key(public_key_a) => ADataPubPermissionSet::new(true, true)],
        entries_index: 0,
        owners_index: 1,
    };

    unwrap!(data.append_permissions(perms_0.clone(), 0));
    unwrap!(data.append(
        vec![ADataEntry {
            key: b"one".to_vec(),
            value: b"foo".to_vec()
        }],
        0
    ));
    unwrap!(data.append(
        vec![ADataEntry {
            key: b"two".to_vec(),
            value: b"foo".to_vec()
        }],
        1
    ));

    common::perform_mutation(
        &mut env,
        &mut client_a,
        Request::PutAData(data.clone().into()),
    );

    common::send_request_expect_ok(
        &mut env,
        &mut client_a,
        Request::GetADataOwners {
            address: *data.address(),
            owners_index: ADataIndex::FromStart(0),
        },
        owner_0,
    );
    // Neither A or B can get the owners with index 1 (it doesn't exist)
    common::send_request_expect_err(
        &mut env,
        &mut client_a,
        Request::GetADataOwners {
            address: *data.address(),
            owners_index: ADataIndex::FromStart(1),
        },
        NdError::InvalidOwners,
    );
    common::send_request_expect_err(
        &mut env,
        &mut client_b,
        Request::GetADataOwners {
            address: *data.address(),
            owners_index: ADataIndex::FromStart(1),
        },
        NdError::InvalidOwners,
    );

    // Set the new owner, change from A -> B
    let owner_1 = ADataOwner {
        public_key: public_key_b,
        entries_index: 2,
        permissions_index: 1,
    };

    // B can't set the new owner, but A can
    common::send_request_expect_err(
        &mut env,
        &mut client_b,
        Request::SetADataOwner {
            address: *data.address(),
            owner: owner_1,
            owners_idx: 1,
        },
        // TODO - InvalidPermissions because client B doesn't have their key registered. Maybe we
        //        should consider changing this.
        NdError::InvalidPermissions,
    );
    common::perform_mutation(
        &mut env,
        &mut client_a,
        Request::SetADataOwner {
            address: *data.address(),
            owner: owner_1,
            owners_idx: 1,
        },
    );

    // Check the new owner
    common::send_request_expect_ok(
        &mut env,
        &mut client_a,
        Request::GetADataOwners {
            address: *data.address(),
            owners_index: ADataIndex::FromStart(1),
        },
        owner_1,
    );
    common::send_request_expect_ok(
        &mut env,
        &mut client_b,
        Request::GetADataOwners {
            address: *data.address(),
            owners_index: ADataIndex::FromStart(1),
        },
        owner_1,
    );
}

#[test]
fn append_only_data_append_seq() {
    let mut env = Environment::new();
    let mut client = env.new_connected_client();
    let public_key = *client.public_id().public_key();

    let start_nano = 1_000_000_000_000;
    common::create_balance_from_nano(&mut env, &mut client, start_nano, None);

    let name: XorName = env.rng().gen();
    let tag = 100;
    let mut data = PubSeqAppendOnlyData::new(name, tag);

    let owner_0 = ADataOwner {
        public_key,
        entries_index: 0,
        permissions_index: 0,
    };
    unwrap!(data.append_owner(owner_0, 0));

    let perms_0 = ADataPubPermissions {
        permissions: btreemap![ADataUser::Anyone => ADataPubPermissionSet::new(true, true)],
        entries_index: 0,
        owners_index: 1,
    };

    unwrap!(data.append_permissions(perms_0.clone(), 0));
    unwrap!(data.append(
        vec![ADataEntry {
            key: b"one".to_vec(),
            value: b"foo".to_vec()
        }],
        0
    ));
    unwrap!(data.append(
        vec![ADataEntry {
            key: b"two".to_vec(),
            value: b"foo".to_vec()
        }],
        1
    ));

    common::perform_mutation(
        &mut env,
        &mut client,
        Request::PutAData(data.clone().into()),
    );

    common::send_request_expect_ok(
        &mut env,
        &mut client,
        Request::GetADataLastEntry(*data.address()),
        ADataEntry::new(b"two".to_vec(), b"foo".to_vec()),
    );

    let appended_values = ADataEntry::new(b"three".to_vec(), b"bar".to_vec());
    let append = ADataAppend {
        address: *data.address(),
        values: vec![appended_values.clone()],
    };
    // First try an invalid append
    common::send_request_expect_err(
        &mut env,
        &mut client,
        Request::AppendUnseq(append.clone()),
        NdError::InvalidOperation,
    );
    common::perform_mutation(
        &mut env,
        &mut client,
        Request::AppendSeq { append, index: 2 },
    );

    // Check the result
    common::send_request_expect_ok(
        &mut env,
        &mut client,
        Request::GetADataLastEntry(*data.address()),
        appended_values,
    );
}

#[test]
fn append_only_data_append_unseq() {
    let mut env = Environment::new();
    let mut client = env.new_connected_client();
    let public_key = *client.public_id().public_key();

    let start_nano = 1_000_000_000_000;
    common::create_balance_from_nano(&mut env, &mut client, start_nano, None);

    let name: XorName = env.rng().gen();
    let tag = 100;
    let mut data = PubUnseqAppendOnlyData::new(name, tag);

    let owner_0 = ADataOwner {
        public_key,
        entries_index: 0,
        permissions_index: 0,
    };
    unwrap!(data.append_owner(owner_0, 0));

    let perms_0 = ADataPubPermissions {
        permissions: btreemap![ADataUser::Anyone => ADataPubPermissionSet::new(true, true)],
        entries_index: 0,
        owners_index: 1,
    };

    unwrap!(data.append_permissions(perms_0.clone(), 0));
    unwrap!(data.append(vec![ADataEntry {
        key: b"one".to_vec(),
        value: b"foo".to_vec()
    }]));
    unwrap!(data.append(vec![ADataEntry {
        key: b"two".to_vec(),
        value: b"foo".to_vec()
    }]));

    common::perform_mutation(
        &mut env,
        &mut client,
        Request::PutAData(data.clone().into()),
    );

    common::send_request_expect_ok(
        &mut env,
        &mut client,
        Request::GetADataLastEntry(*data.address()),
        ADataEntry::new(b"two".to_vec(), b"foo".to_vec()),
    );

    let appended_values = ADataEntry::new(b"three".to_vec(), b"bar".to_vec());
    let append = ADataAppend {
        address: *data.address(),
        values: vec![appended_values.clone()],
    };

    // First try an invalid append
    common::send_request_expect_err(
        &mut env,
        &mut client,
        Request::AppendSeq {
            append: append.clone(),
            index: 2,
        },
        NdError::InvalidOperation,
    );
    common::perform_mutation(&mut env, &mut client, Request::AppendUnseq(append));

    // Check the result
    common::send_request_expect_ok(
        &mut env,
        &mut client,
        Request::GetADataLastEntry(*data.address()),
        appended_values,
    );
}

////////////////////////////////////////////////////////////////////////////////
//
// Immutable data
//
////////////////////////////////////////////////////////////////////////////////

#[test]
fn put_immutable_data() {
    let mut env = Environment::new();

    let mut client_a = env.new_connected_client();
    let mut client_b = env.new_connected_client();

    let mut raw_data = vec![0u8; 1024];
    env.rng().fill(raw_data.as_mut_slice());
    let pub_idata = IData::Pub(PubImmutableData::new(raw_data.clone()));
    let unpub_idata = IData::Unpub(UnpubImmutableData::new(
        raw_data,
        *client_b.public_id().public_key(),
    ));

    // TODO - enable this once we're passed phase 1.
    if false {
        // Put should fail when the client has no associated balance.
        common::send_request_expect_err(
            &mut env,
            &mut client_a,
            Request::PutIData(pub_idata.clone()),
            NdError::InsufficientBalance,
        );
        common::send_request_expect_err(
            &mut env,
            &mut client_b,
            Request::PutIData(unpub_idata.clone()),
            NdError::InsufficientBalance,
        );
    }

    // Create balances.  Client A starts with 2000 safecoins and spends 1000 to initialise
    // Client B's balance.
    let start_nano = 1_000_000_000_000;
    common::create_balance_from_nano(&mut env, &mut client_a, start_nano * 2, None);
    common::create_balance_from_nano(
        &mut env,
        &mut client_a,
        start_nano,
        Some(*client_b.public_id().public_key()),
    );

    // Check client A can't Put an UnpubIData where B is the owner.
    common::send_request_expect_err(
        &mut env,
        &mut client_a,
        Request::PutIData(unpub_idata.clone()),
        NdError::InvalidOwners,
    );

    let mut expected = unwrap!(Coins::from_nano(start_nano));
    common::send_request_expect_ok(&mut env, &mut client_a, Request::GetBalance, expected);

    for _ in &[0, 1] {
        // Check they can both Put valid data.
        common::perform_mutation(
            &mut env,
            &mut client_a,
            Request::PutIData(pub_idata.clone()),
        );
        common::perform_mutation(
            &mut env,
            &mut client_b,
            Request::PutIData(unpub_idata.clone()),
        );

        expected = unwrap!(expected.checked_sub(*COST_OF_PUT));
        common::send_request_expect_ok(&mut env, &mut client_a, Request::GetBalance, expected);
        common::send_request_expect_ok(&mut env, &mut client_b, Request::GetBalance, expected);

        // Check the data is retrievable.
        common::send_request_expect_ok(
            &mut env,
            &mut client_a,
            Request::GetIData(*pub_idata.address()),
            pub_idata.clone(),
        );
        common::send_request_expect_ok(
            &mut env,
            &mut client_b,
            Request::GetIData(*unpub_idata.address()),
            unpub_idata.clone(),
        );
    }
}

#[test]
fn get_immutable_data_that_doesnt_exist() {
    let mut env = Environment::new();
    let mut client = env.new_connected_client();

    // Try to get non-existing published immutable data
    let address: XorName = env.rng().gen();
    common::send_request_expect_err(
        &mut env,
        &mut client,
        Request::GetIData(IDataAddress::Pub(address)),
        NdError::NoSuchData,
    );

    // TODO - enable this once we're passed phase 1.
    if false {
        // Try to get non-existing unpublished immutable data while having no balance
        common::send_request_expect_err(
            &mut env,
            &mut client,
            Request::GetIData(IDataAddress::Unpub(address)),
            NdError::AccessDenied,
        );
    }

    // Try to get non-existing unpublished immutable data while having balance
    let start_nano = 1_000_000_000_000;
    common::create_balance_from_nano(&mut env, &mut client, start_nano, None);
    common::send_request_expect_err(
        &mut env,
        &mut client,
        Request::GetIData(IDataAddress::Unpub(address)),
        NdError::NoSuchData,
    );
}

#[test]
fn get_immutable_data_from_other_owner() {
    let mut env = Environment::new();

    let mut client_a = env.new_connected_client();
    let mut client_b = env.new_connected_client();

    let start_nano = 1_000_000_000_000;
    common::create_balance_from_nano(&mut env, &mut client_a, start_nano, None);
    common::create_balance_from_nano(&mut env, &mut client_b, start_nano, None);

    // Client A uploads published data that Client B can fetch
    let pub_idata = IData::Pub(PubImmutableData::new(vec![1, 2, 3]));
    let mut request = Request::GetIData(*pub_idata.address());
    common::perform_mutation(
        &mut env,
        &mut client_a,
        Request::PutIData(pub_idata.clone()),
    );
    common::send_request_expect_ok(&mut env, &mut client_a, request.clone(), pub_idata.clone());
    common::send_request_expect_ok(&mut env, &mut client_b, request, pub_idata);

    // Client A uploads unpublished data that Client B can't fetch
    let owner = client_a.public_id().public_key();
    let unpub_idata = IData::Unpub(UnpubImmutableData::new(vec![42], *owner));
    request = Request::GetIData(*unpub_idata.address());
    common::perform_mutation(
        &mut env,
        &mut client_a,
        Request::PutIData(unpub_idata.clone()),
    );
    common::send_request_expect_ok(&mut env, &mut client_a, request.clone(), unpub_idata);
    common::send_request_expect_err(&mut env, &mut client_b, request, NdError::AccessDenied);
}

#[test]
fn put_pub_and_get_unpub_immutable_data_at_same_xor_name() {
    let mut env = Environment::new();
    let mut client = env.new_connected_client();

    // Create balance.
    let start_nano = 1_000_000_000_000;
    common::create_balance_from_nano(&mut env, &mut client, start_nano, None);

    // Put and verify some published immutable data
    let pub_idata = IData::Pub(PubImmutableData::new(vec![1, 2, 3]));
    let pub_idata_address: XorName = *pub_idata.address().name();
    common::perform_mutation(&mut env, &mut client, Request::PutIData(pub_idata.clone()));
    assert_eq!(
        pub_idata,
        common::get_from_response(
            &mut env,
            &mut client,
            Request::GetIData(IDataAddress::Pub(pub_idata_address))
        ),
    );

    // Get some unpublished immutable data from the same address
    common::send_request_expect_err(
        &mut env,
        &mut client,
        Request::GetIData(IDataAddress::Unpub(pub_idata_address)),
        NdError::NoSuchData,
    );
}

#[test]
fn put_unpub_and_get_pub_immutable_data_at_same_xor_name() {
    let mut env = Environment::new();
    let mut client = env.new_connected_client();

    // Create balances.
    let start_nano = 1_000_000_000_000;
    common::create_balance_from_nano(&mut env, &mut client, start_nano, None);

    // Put and verify some unpub immutable data
    let owner = client.public_id().public_key();
    let unpub_idata = IData::Unpub(UnpubImmutableData::new(vec![1, 2, 3], *owner));
    let unpub_idata_address: XorName = *unpub_idata.address().name();
    common::perform_mutation(
        &mut env,
        &mut client,
        Request::PutIData(unpub_idata.clone()),
    );
    assert_eq!(
        unpub_idata,
        common::get_from_response(
            &mut env,
            &mut client,
            Request::GetIData(IDataAddress::Unpub(unpub_idata_address))
        ),
    );

    // Get some published immutable data from the same address
    common::send_request_expect_err(
        &mut env,
        &mut client,
        Request::GetIData(IDataAddress::Pub(unpub_idata_address)),
        NdError::NoSuchData,
    );
}

#[test]
fn delete_immutable_data_that_doesnt_exist() {
    let mut env = Environment::new();
    let mut client = env.new_connected_client();

    // Try to delete non-existing published idata while not having a balance
    let address: XorName = env.rng().gen();
    common::send_request_expect_err(
        &mut env,
        &mut client,
        Request::DeleteUnpubIData(IDataAddress::Pub(address)),
        NdError::InvalidOperation,
    );

    // TODO - enable this once we're passed phase 1.
    if false {
        // Try to delete non-existing unpublished data while not having a balance
        common::send_request_expect_err(
            &mut env,
            &mut client,
            Request::GetIData(IDataAddress::Unpub(address)),
            NdError::AccessDenied,
        );
    }

    // Try to delete non-existing unpublished data
    let start_nano = 1_000_000_000_000;
    common::create_balance_from_nano(&mut env, &mut client, start_nano, None);
    common::send_request_expect_err(
        &mut env,
        &mut client,
        Request::GetIData(IDataAddress::Unpub(address)),
        NdError::NoSuchData,
    );
}

#[test]
fn delete_immutable_data() {
    let mut env = Environment::new();
    let mut client_a = env.new_connected_client();
    let mut client_b = env.new_connected_client();

    let start_nano = 1_000_000_000_000;
    common::create_balance_from_nano(&mut env, &mut client_a, start_nano, None);

    let raw_data = vec![1, 2, 3];
    let pub_idata = IData::Pub(PubImmutableData::new(raw_data.clone()));
    let pub_idata_address: XorName = *pub_idata.address().name();
    common::perform_mutation(&mut env, &mut client_a, Request::PutIData(pub_idata));

    // Try to delete published data by constructing inconsistent Request
    common::send_request_expect_err(
        &mut env,
        &mut client_a,
        Request::DeleteUnpubIData(IDataAddress::Pub(pub_idata_address)),
        NdError::InvalidOperation,
    );

    // Try to delete published data by raw XorName
    common::send_request_expect_err(
        &mut env,
        &mut client_a,
        Request::DeleteUnpubIData(IDataAddress::Unpub(pub_idata_address)),
        NdError::NoSuchData,
    );

    let raw_data = vec![42];
    let owner = client_a.public_id().public_key();
    let unpub_idata = IData::Unpub(UnpubImmutableData::new(raw_data.clone(), *owner));
    let unpub_idata_address: XorName = *unpub_idata.address().name();
    common::perform_mutation(&mut env, &mut client_a, Request::PutIData(unpub_idata));

    // TODO - enable this once we're passed phase 1.
    if false {
        // Delete unpublished data without being the owner
        common::send_request_expect_err(
            &mut env,
            &mut client_b,
            Request::DeleteUnpubIData(IDataAddress::Unpub(unpub_idata_address)),
            NdError::AccessDenied,
        );
    }

    // Delete unpublished data without having the balance
    common::perform_mutation(
        &mut env,
        &mut client_a,
        Request::DeleteUnpubIData(IDataAddress::Unpub(unpub_idata_address)),
    );

    // Delete unpublished data again
    common::send_request_expect_err(
        &mut env,
        &mut client_a,
        Request::DeleteUnpubIData(IDataAddress::Unpub(unpub_idata_address)),
        NdError::NoSuchData,
    )
}

////////////////////////////////////////////////////////////////////////////////
//
// Auth keys
//
////////////////////////////////////////////////////////////////////////////////

#[test]
fn auth_keys() {
    type KeysResult = NdResult<(BTreeMap<PublicKey, AppPermissions>, u64)>;
    fn list_keys<T: TestClientTrait>(env: &mut Environment, client: &mut T, expected: KeysResult) {
        let request = Request::ListAuthKeysAndVersion;
        match expected {
            Ok(expected) => common::send_request_expect_ok(env, client, request, expected),
            Err(expected) => common::send_request_expect_err(env, client, request, expected),
        }
    }

    let mut env = Environment::new();
    let mut owner = env.new_connected_client();
    let mut app = env.new_connected_app(owner.public_id().clone());

    let permissions = AppPermissions {
        transfer_coins: true,
    };
    let app_public_key = *app.public_id().public_key();
    let make_ins_request = |version| Request::InsAuthKey {
        key: app_public_key,
        version,
        permissions,
    };

    // TODO - enable this once we're passed phase 1.
    if false {
        // Try to insert and then list authorised keys using a client with no balance.  Each should
        // return `NoSuchBalance`.
        common::send_request_expect_err(
            &mut env,
            &mut owner,
            make_ins_request(1),
            NdError::NoSuchBalance,
        );
        list_keys(&mut env, &mut owner, Err(NdError::NoSuchBalance));
    }

    // Create a balance for the owner and check that listing authorised keys returns an empty
    // collection.
    let start_nano = 1_000_000_000_000;
    common::create_balance_from_nano(&mut env, &mut owner, start_nano, None);
    let mut expected_map = BTreeMap::new();
    list_keys(&mut env, &mut owner, Ok((expected_map.clone(), 0)));

    // Insert then list the app.
    let _ = expected_map.insert(*app.public_id().public_key(), permissions);
    common::perform_mutation(&mut env, &mut owner, make_ins_request(1));
    list_keys(&mut env, &mut owner, Ok((expected_map.clone(), 1)));

    // Check the app isn't allowed to get a listing of authorised keys, nor insert, nor delete any.
    // No response should be returned to any of these requests.
    common::send_request_expect_no_response(&mut env, &mut app, Request::ListAuthKeysAndVersion);
    common::send_request_expect_no_response(&mut env, &mut app, make_ins_request(2));
    let del_auth_key_request = Request::DelAuthKey {
        key: *app.public_id().public_key(),
        version: 2,
    };
    common::send_request_expect_no_response(&mut env, &mut app, del_auth_key_request.clone());

    // Remove the app, then list the keys.
    common::perform_mutation(&mut env, &mut owner, del_auth_key_request);
    list_keys(&mut env, &mut owner, Ok((BTreeMap::new(), 2)));

    // Try to insert using an invalid version number.
    common::send_request_expect_err(
        &mut env,
        &mut owner,
        make_ins_request(100),
        NdError::InvalidSuccessor(2),
    );
    list_keys(&mut env, &mut owner, Ok((BTreeMap::new(), 2)));

    // Insert again and list again.
    common::perform_mutation(&mut env, &mut owner, make_ins_request(3));
    list_keys(&mut env, &mut owner, Ok((expected_map, 3)));
}

////////////////////////////////////////////////////////////////////////////////
//
// Mutable data
//
////////////////////////////////////////////////////////////////////////////////

#[test]
fn put_seq_mutable_data() {
    let mut env = Environment::new();
    let mut client = env.new_connected_client();

    common::create_balance_from_nano(&mut env, &mut client, 0, None);

    // Try to put sequenced Mutable Data
    let name: XorName = env.rng().gen();
    let tag = 100;
    let mdata = SeqMutableData::new(name, tag, *client.public_id().public_key());
    common::perform_mutation(
        &mut env,
        &mut client,
        Request::PutMData(MData::Seq(mdata.clone())),
    );

    // Get Mutable Data and verify it's been stored correctly.
    common::send_request_expect_ok(
        &mut env,
        &mut client,
        Request::GetMData(MDataAddress::Seq { name, tag }),
        MData::Seq(mdata),
    );
}

#[test]
fn put_unseq_mutable_data() {
    let mut env = Environment::new();
    let mut client = env.new_connected_client();

    common::create_balance_from_nano(&mut env, &mut client, 0, None);

    // Try to put unsequenced Mutable Data
    let name: XorName = env.rng().gen();
    let tag = 100;
    let mdata = UnseqMutableData::new(name, tag, *client.public_id().public_key());
    common::perform_mutation(
        &mut env,
        &mut client,
        Request::PutMData(MData::Unseq(mdata.clone())),
    );

    // Get Mutable Data and verify it's been stored correctly.
    common::send_request_expect_ok(
        &mut env,
        &mut client,
        Request::GetMData(MDataAddress::Unseq { name, tag }),
        MData::Unseq(mdata),
    );
}

#[test]
fn read_seq_mutable_data() {
    let mut env = Environment::new();
    let mut client = env.new_connected_client();

    common::create_balance_from_nano(&mut env, &mut client, 0, None);

    // Try to put sequenced Mutable Data with several entries.
    let entries: BTreeMap<_, _> = (1..4)
        .map(|_| {
            let key = env.rng().sample_iter(&Standard).take(8).collect();
            let data = env.rng().sample_iter(&Standard).take(8).collect();
            (key, MDataValue { data, version: 0 })
        })
        .collect();

    let name: XorName = env.rng().gen();
    let tag = 100;
    let mdata = SeqMutableData::new_with_data(
        name,
        tag,
        entries.clone(),
        Default::default(),
        *client.public_id().public_key(),
    );
    common::perform_mutation(
        &mut env,
        &mut client,
        Request::PutMData(MData::Seq(mdata.clone())),
    );

    // Get version.
    let address = MDataAddress::Seq { name, tag };
    common::send_request_expect_ok(&mut env, &mut client, Request::GetMDataVersion(address), 0);

    // Get keys.
    common::send_request_expect_ok(
        &mut env,
        &mut client,
        Request::ListMDataKeys(address),
        entries.keys().cloned().collect::<BTreeSet<_>>(),
    );

    // Get values.
    common::send_request_expect_ok(
        &mut env,
        &mut client,
        Request::ListMDataValues(address),
        entries.values().cloned().collect::<Vec<_>>(),
    );

    // Get entries.
    common::send_request_expect_ok(
        &mut env,
        &mut client,
        Request::ListMDataEntries(address),
        entries.clone(),
    );

    // Get a value by key.
    let key = unwrap!(entries.keys().cloned().nth(0));
    common::send_request_expect_ok(
        &mut env,
        &mut client,
        Request::GetMDataValue {
            address,
            key: key.clone(),
        },
        entries[&key].clone(),
    );
}

#[test]
fn mutate_seq_mutable_data() {
    let mut env = Environment::new();
    let mut client = env.new_connected_client();

    common::create_balance_from_nano(&mut env, &mut client, 0, None);

    // Try to put sequenced Mutable Data.
    let name: XorName = env.rng().gen();
    let tag = 100;
    let mdata = SeqMutableData::new(name, tag, *client.public_id().public_key());
    common::perform_mutation(
        &mut env,
        &mut client,
        Request::PutMData(MData::Seq(mdata.clone())),
    );

    // Get a non-existant value by key.
    let address = MDataAddress::Seq { name, tag };
    common::send_request_expect_err(
        &mut env,
        &mut client,
        Request::GetMDataValue {
            address,
            key: vec![0],
        },
        NdError::NoSuchEntry,
    );

    // Insert new values.
    let actions = MDataSeqEntryActions::new()
        .ins(vec![0], vec![1], 0)
        .ins(vec![1], vec![1], 0);
    common::perform_mutation(
        &mut env,
        &mut client,
        Request::MutateSeqMDataEntries { address, actions },
    );

    // Get an existing value by key.
    common::send_request_expect_ok(
        &mut env,
        &mut client,
        Request::GetMDataValue {
            address,
            key: vec![0],
        },
        MDataValue {
            data: vec![1],
            version: 0,
        },
    );

    // Update and delete entries.
    let actions = MDataSeqEntryActions::new()
        .update(vec![0], vec![2], 1)
        .del(vec![1], 1);
    common::perform_mutation(
        &mut env,
        &mut client,
        Request::MutateSeqMDataEntries { address, actions },
    );

    // Get an existing value by key.
    common::send_request_expect_ok(
        &mut env,
        &mut client,
        Request::GetMDataValue {
            address,
            key: vec![0],
        },
        MDataValue {
            data: vec![2],
            version: 1,
        },
    );

    // Deleted key should not exist now.
    common::send_request_expect_err(
        &mut env,
        &mut client,
        Request::GetMDataValue {
            address,
            key: vec![1],
        },
        NdError::NoSuchEntry,
    );

    // Try an invalid update request.
    let expected_invalid_actions = btreemap![vec![0] => EntryError::InvalidSuccessor(1)];
    let actions = MDataSeqEntryActions::new().update(vec![0], vec![3], 0);
    common::send_request_expect_err(
        &mut env,
        &mut client,
        Request::MutateSeqMDataEntries {
            address: MDataAddress::Seq { name, tag },
            actions,
        },
        NdError::InvalidEntryActions(expected_invalid_actions),
    );
}

#[test]
fn mutate_unseq_mutable_data() {
    let mut env = Environment::new();
    let mut client = env.new_connected_client();

    common::create_balance_from_nano(&mut env, &mut client, 0, None);

    // Try to put unsequenced Mutable Data.
    let name: XorName = env.rng().gen();
    let tag = 100;
    let mdata = UnseqMutableData::new(name, tag, *client.public_id().public_key());
    common::perform_mutation(
        &mut env,
        &mut client,
        Request::PutMData(MData::Unseq(mdata.clone())),
    );

    // Get a non-existant value by key.
    let address = MDataAddress::Unseq { name, tag };
    common::send_request_expect_err(
        &mut env,
        &mut client,
        Request::GetMDataValue {
            address,
            key: vec![0],
        },
        NdError::NoSuchEntry,
    );

    // Insert new values.
    let actions = MDataUnseqEntryActions::new()
        .ins(vec![0], vec![1])
        .ins(vec![1], vec![1]);
    common::perform_mutation(
        &mut env,
        &mut client,
        Request::MutateUnseqMDataEntries { address, actions },
    );

    // Get an existing value by key.
    common::send_request_expect_ok(
        &mut env,
        &mut client,
        Request::GetMDataValue {
            address,
            key: vec![0],
        },
        vec![1],
    );

    // Update and delete entries.
    let actions = MDataUnseqEntryActions::new()
        .update(vec![0], vec![2])
        .del(vec![1]);
    common::perform_mutation(
        &mut env,
        &mut client,
        Request::MutateUnseqMDataEntries { address, actions },
    );

    // Get an existing value by key.
    common::send_request_expect_ok(
        &mut env,
        &mut client,
        Request::GetMDataValue {
            address,
            key: vec![0],
        },
        vec![2],
    );

    // Deleted key should not exist now.
    common::send_request_expect_err(
        &mut env,
        &mut client,
        Request::GetMDataValue {
            address,
            key: vec![1],
        },
        NdError::NoSuchEntry,
    );
}

#[test]
fn mutable_data_permissions() {
    let mut env = Environment::new();

    let mut client_a = env.new_connected_client();
    let mut client_b = env.new_connected_client();

    common::create_balance_from_nano(&mut env, &mut client_a, 0, None);
    common::create_balance_from_nano(&mut env, &mut client_b, 0, None);

    // Try to put new unsequenced Mutable Data.
    let name: XorName = env.rng().gen();
    let tag = 100;
    let mdata = UnseqMutableData::new(name, tag, *client_a.public_id().public_key());
    common::perform_mutation(
        &mut env,
        &mut client_a,
        Request::PutMData(MData::Unseq(mdata.clone())),
    );

    // Make sure client B can't insert anything.
    let actions = MDataUnseqEntryActions::new().ins(vec![0], vec![1]);
    let address = MDataAddress::Unseq { name, tag };
    common::send_request_expect_err(
        &mut env,
        &mut client_b,
        Request::MutateUnseqMDataEntries { address, actions },
        NdError::AccessDenied,
    );

    // Insert permissions for client B.
    common::perform_mutation(
        &mut env,
        &mut client_a,
        Request::SetMDataUserPermissions {
            address,
            user: *client_b.public_id().public_key(),
            permissions: MDataPermissionSet::new().allow(MDataAction::Insert),
            version: 1,
        },
    );

    // Client B now can insert new values.
    let actions = MDataUnseqEntryActions::new().ins(vec![0], vec![1]);
    common::perform_mutation(
        &mut env,
        &mut client_b,
        Request::MutateUnseqMDataEntries { address, actions },
    );

    // Delete client B permissions.
    common::perform_mutation(
        &mut env,
        &mut client_a,
        Request::DelMDataUserPermissions {
            address,
            user: *client_b.public_id().public_key(),
            version: 2,
        },
    );

    // Client B can't insert anything again.
    let actions = MDataUnseqEntryActions::new().ins(vec![0], vec![1]);
    common::send_request_expect_err(
        &mut env,
        &mut client_b,
        Request::MutateUnseqMDataEntries { address, actions },
        NdError::AccessDenied,
    );
}
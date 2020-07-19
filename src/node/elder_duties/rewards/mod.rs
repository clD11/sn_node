// Copyright 2020 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

mod section_funds;
mod system;
mod validator;

use self::section_funds::SectionFunds;
pub use self::{system::FarmingSystem, validator::Validator};
use crate::{node::node_ops::{NodeDuty, NodeOperation, MessagingDuty, RewardDuty}, node::keys::NodeKeys, node::msg_wrapping::ElderMsgWrapping};
use safe_farming::{Accumulation, StorageRewards};
use safe_nd::{
    AccountId, ElderDuties, Error, Message, MessageId, Money, Address, NetworkCmd, NetworkCmdError,
    NetworkEvent, NetworkRewardError, RewardCounter, XorName, NetworkRewardCmd,
};
use safe_transfers::TransferActor;
use std::collections::HashMap;

pub struct Rewards {
    farming: FarmingSystem<StorageRewards>,
    node_accounts: HashMap<XorName, RewardAccount>,
    section_funds: SectionFunds,
    decisions: ElderMsgWrapping,
}

#[derive(PartialEq)]
pub enum RewardAccount {
    /// When added.
    AwaitingStart,
    /// After having received the counter, the
    /// stage of the RewardAccount is `Active`.
    Active(AccountId),
    /// After a node leaves the section
    /// the RewardAccount transitions into
    /// stage `AwaitingMove`.
    AwaitingMove(AccountId),
}

impl Rewards {
    pub fn new(keys: NodeKeys, actor: TransferActor<Validator>) -> Self {
        let decisions = ElderMsgWrapping::new(keys, ElderDuties::Rewards);
        let acc = Accumulation::new(Default::default(), Default::default());
        let base_cost = Money::from_nano(1);
        let algo = StorageRewards::new(base_cost);
        let farming = FarmingSystem::new(algo, acc);
        let section_funds = SectionFunds::new(actor, decisions.clone());
        Self {
            farming,
            node_accounts: Default::default(),
            section_funds,
            decisions,
        }
    }

    pub fn process(&mut self, duty: RewardDuty) -> Option<NodeOperation> {
        use RewardDuty::*;
        let result = match duty {
            AccumulateReward {
                data,
            } => self.accumulate_reward(data),
            AddNewAccount {
                id, 
                node_id,
            } => self.add_account(id, node_id),
            AddRelocatedAccount {
                old_node_id,
                new_node_id,
            } => self.add_relocated_account(old_node_id, new_node_id),
            ClaimRewardCounter {
                old_node_id, new_node_id, msg_id, origin,
            } => self.claim_rewards(old_node_id, new_node_id, msg_id, &origin),
            ReceiveClaimedRewards {
                id,
                node_id,
                counter,
            } => self.receive_claimed_rewards(id, node_id, counter),
            PrepareAccountMove {
                node_id,
            } => self.node_left(node_id)
        };
        use NodeDuty::*;
        use NodeOperation::*;
        
        result.map(|c| RunAsNode(ProcessMessaging(c)))
    }

    /// 0. A brand new node has joined our section.
    fn add_account(&mut self, id: AccountId, node_id: XorName) -> Option<MessagingDuty> {
        let _ = self.node_accounts.insert(node_id, RewardAccount::Active(id));
        None
    }

    /// 1. When a node is relocated to our section, we add the account
    /// and send a cmd to old section, for claiming the rewards.
    fn add_relocated_account(
        &mut self,
        old_node_id: XorName,
        new_node_id: XorName,
    ) -> Option<MessagingDuty> {
        use NetworkCmd::*;
        use NetworkRewardCmd::*;

        let _ = self
            .node_accounts
            .insert(new_node_id, RewardAccount::AwaitingStart);

        self.decisions.send(Message::NetworkCmd {
            cmd: Rewards(ClaimRewardCounter {
                old_node_id,
                new_node_id,
            }),
            id: MessageId::new(),
        })
    }

    /// 2. The old section will send back the claimed rewards.
    /// Work is the total work associated with this account id.
    /// It is a strictly incrementing value during the lifetime of
    /// the owner on the network.
    fn receive_claimed_rewards(
        &mut self,
        id: AccountId,
        node_id: XorName,
        counter: RewardCounter,
    ) -> Option<MessagingDuty> {
        // TODO: Consider this validation code here, and the flow..
        // .. because we are receiving an event triggered by our cmd, something is very odd
        // most likely a bug, if we ever hit these errors.
        // So, it doesn't make much sense to send some error msg on the wire.
        // Makes more sense to panic, or log and just drop the request.
        // But exact course to take there needs to be chiseled out.

        // Try get the account..
        match self.node_accounts.get(&node_id) {
            None => {
                // "Invalid receive: No such account found to receive the rewards.".to_string()
                return None;
            }
            Some(account) => {
                // ..and validate its state.
                if *account != RewardAccount::AwaitingStart {
                    // "Invalid receive: Account is not awaiting start.".to_string()
                    return None;
                }
            }
        };

        // Add the account to our farming.
        match self.farming.add_account(id, counter.work) {
            Ok(_) => {
                // Set the stage to `Active`
                let _ = self
                    .node_accounts
                    .insert(node_id, RewardAccount::Active(id));
                // If any reward was accumulated,
                // we initiate payout to the account.
                if counter.reward > Money::zero() {
                    return self
                        .section_funds
                        .initiate_reward_payout(counter.reward, id);
                }
                None
            }
            Err(_error) => {
                // Really, the same comment about error
                // as above, applies here as well..
                // There is nothing the old section can do about this error
                // and it should be a bug, so, something other than sending
                // an error to the old section needs to be done here.

                // "Failed to receive! Error: {_error}.".to_string()
                None
            }
        }
    }

    /// 3. Every time the section receives
    /// a write request, the accounts accumulate reward.
    fn accumulate_reward(&mut self, data: Vec<u8>) -> Option<MessagingDuty> {
        let num_bytes = data.len() as u64;
        let data_hash = data;
        let factor = 2.0;
        match self.farming.reward(data_hash, num_bytes, factor) {
            Ok(_) => None,
            Err(_err) => None, // todo: NetworkCmdError. Or not? This is an internal thing..
        }
    }

    /// 4. When the section becomes aware that a node has left,
    /// it is flagged for being awaiting move.
    fn node_left(&mut self, node_id: XorName) -> Option<MessagingDuty> {
        let id = match self.node_accounts.get(&node_id) {
            Some(RewardAccount::Active(id)) => *id,
            Some(RewardAccount::AwaitingStart) // hmm.. left when AwaitingStart is a tricky case..
            | Some(RewardAccount::AwaitingMove(_))
            | None => return None,
        };
        let _ = self
            .node_accounts
            .insert(node_id, RewardAccount::AwaitingMove(id));
        None
    }

    /// 5. The section that received a relocated node,
    /// will locally be executing `add_account(..)` of this very module,
    /// thereby sending a cmd to the old section, leading to this method
    /// here being called. An event will be sent back with the claimed counter.
    fn claim_rewards(
        &mut self,
        old_node_id: XorName,
        new_node_id: XorName,
        msg_id: MessageId,
        origin: &Address,
    ) -> Option<MessagingDuty> {
        use NetworkCmdError::*;
        use NetworkRewardError::*;

        let account_id = match self.node_accounts.get(&old_node_id) {
            Some(RewardAccount::AwaitingMove(id)) => *id,
            Some(RewardAccount::Active(id)) => {
                // ..means the node has not left, and was not
                // marked as awaiting move..
                return self.decisions.network_error(
                    Rewards(RewardClaiming {
                        error: Error::NetworkOther(
                            "InvalidClaim: Account is not awaiting move.".to_string(),
                        ),
                        account_id: *id,
                    }),
                    msg_id,
                    *origin,
                );
            }
            Some(RewardAccount::AwaitingStart) // todo: return error, but we need to have the account id in that case, or change / extend the current error(s)
            | None => return None,
        };

        // Claim the counter. (This removes it from our state.)
        let counter = match self.farming.claim(account_id) {
            Ok(counter) => counter,
            Err(error) => {
                return self.decisions.network_error(
                    Rewards(RewardClaiming { error, account_id }),
                    msg_id,
                    *origin,
                );
            }
        };

        // Remove the old node, as it is being
        // taken over by the new section.
        let _ = self.node_accounts.remove(&old_node_id);

        // Send the reward counter to the new section.
        // Once received over there, the new section
        // will pay out any accumulated rewards to the account.
        // From there on, they accumulate rewards for the node
        // until it is being relocated again.
        self.decisions.send(Message::NetworkEvent {
            event: NetworkEvent::RewardCounterClaimed {
                new_node_id,
                account_id,
                counter,
            },
            id: MessageId::new(),
            correlation_id: msg_id,
        })
    }
}

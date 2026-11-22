//! Host-owned profile transactions. Clock receipts are process-local, while
//! history promotion deduplicates against persisted attempt identities.
use super::*;

#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub(super) struct ProfileClockCredits {
    // TODO(profile-time): these receipts are intentionally process-local.
    // Durable exactly-once time accounting across application restarts needs
    // a versioned profile/save ledger, not an inferred mission timestamp.
    receipts: Vec<ClockReceipt>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct ClockReceipt {
    profile_id: u32,
    mission_id: u32,
    seconds: u32,
}

impl ProfileClockCredits {
    fn credit(&mut self, profile_id: u32, mission_id: u32, seconds: u32) -> u32 {
        if let Some(receipt) = self
            .receipts
            .iter_mut()
            .find(|receipt| receipt.profile_id == profile_id && receipt.mission_id == mission_id)
        {
            let credit = seconds.saturating_sub(receipt.seconds);
            // Loading/rewinding an earlier clock must not credit the same
            // interval twice. Only mission bootstrap opens a new epoch;
            // appending terminal history is not a new running mission.
            receipt.seconds = receipt.seconds.max(seconds);
            credit
        } else {
            self.receipts.push(ClockReceipt {
                profile_id,
                mission_id,
                seconds,
            });
            seconds
        }
    }
}

/// Import only records that already exist in the authoritative campaign.
/// In particular, a pre-terminal synchronization cannot create the pending
/// attempt. The terminal path calls this again after append and attestation.
///
/// A conflict between durable lifetime history and the canonical campaign is a
/// data-integrity diagnostic, not a control-flow failure. Log it and keep the
/// in-memory profile for the next save/synchronization to retry, the same
/// policy used for persistence failures below. Aborting at terminal debriefing
/// would throw away the whole session over one bad attempt (issue #74).
fn promote_recorded_history(
    profile: &mut robin_engine::player_profile::PlayerProfile,
    campaign: &Campaign,
    profiles: &ProfileManager,
) {
    if let Err(error) = profile.promote_campaign_history(campaign, profiles) {
        tracing::error!(
            "Campaign-history promotion failed; retained the profile in memory and will retry on the next profile save/synchronization: {error}"
        );
    }
}

pub(super) fn synchronize_metrics(
    application: &ApplicationContext,
    credits: &mut ProfileClockCredits,
    campaign: &Campaign,
    profiles: &ProfileManager,
    seconds: u32,
) {
    let mut staged_credits = credits.clone();
    let result = application
        .update_and_retain_player_profiles(|manager| {
            let profile = manager
                .get_active_mut()
                .expect("profile synchronization requires an active profile");
            let credit =
                staged_credits.credit(profile.id, current_mission_id(campaign, profiles), seconds);
            robin_engine::player_profile::synchronize_with_campaign(
                profile, campaign, profiles, credit,
            );
            // Loading an existing campaign then quitting may never enter
            // terminal debriefing. Preserve its already-recorded history here.
            promote_recorded_history(profile, campaign, profiles);
            // Always retry persistence, even if this invocation added no time.
            // The receipt tracks the in-memory application, not disk success.
        })
        .unwrap_or_else(|error| {
            panic!("profile synchronization lost its ApplicationContext: {error}")
        });
    *credits = staged_credits;
    if let Err(error) = result.persistence {
        let kind = super::super::init::classify_profile_rejection(&error);
        crate::campaign_persistence::report_failure(kind, "profile synchronization");
        tracing::error!(
            "Profile synchronization persistence failed; retained in memory and retried on the next profile save/synchronization: {error}"
        );
    }
}

impl RustCallbacks {
    /// Open one running-mission clock epoch, including failed bootstrap.
    /// Same-session restore/restart receipts retain their high-water mark;
    /// authoritative teardown and fresh bootstrap explicitly start over.
    pub(crate) fn begin_profile_clock_session(&mut self) {
        self.profile_clock_credits = ProfileClockCredits::default();
    }

    /// Call only after the terminal engine command and eligibility attestation.
    /// Repeating promotion refreshes attestations without duplicating history;
    /// persistence must run even when promotion reports no new attempts.
    pub(crate) fn promote_terminal_profile(
        application: &ApplicationContext,
        campaign: &Campaign,
        profiles: &ProfileManager,
    ) {
        let promote = |manager: &mut robin_engine::player_profile::PlayerProfileManager| {
            promote_recorded_history(
                manager
                    .get_active_mut()
                    .expect("terminal promotion requires an active profile"),
                campaign,
                profiles,
            );
        };
        let result = if cfg!(target_arch = "wasm32") {
            application.update_and_retain_player_profiles(promote)
        } else {
            application.try_persist_player_profiles(|manager| {
                promote(manager);
                Ok(())
            })
        }
        .unwrap_or_else(|error| panic!("campaign profile synchronization failed: {error}"));
        if let Err(error) = result.persistence {
            #[cfg(not(target_arch = "wasm32"))]
            panic!("failed to persist campaign history: {error}");
            #[cfg(target_arch = "wasm32")]
            {
                let kind = super::super::init::classify_profile_rejection(&error);
                crate::campaign_persistence::report_failure(kind, "campaign history promotion");
                tracing::warn!(
                    "Failed to persist campaign history in browser storage; keeping it in memory for this session: {error}"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_sync_and_load_back_only_credit_new_clock_intervals() {
        let mut credits = ProfileClockCredits::default();
        assert_eq!(credits.credit(1, 17, 60), 60);
        assert_eq!(credits.credit(1, 17, 60), 0);
        assert_eq!(credits.credit(1, 17, 20), 0);
        assert_eq!(credits.credit(1, 17, 70), 10);
        assert_eq!(credits.credit(2, 17, 70), 70);
        assert_eq!(credits.credit(1, 18, 70), 70);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn loaded_campaign_history_survives_quit_sync_and_persistence_retry() {
        let directory = tempfile::tempdir().unwrap();
        let (mut callbacks, _host, engine, _assets, _game, profiles) =
            super::super::operation_outcome_tests::diagnostic_callback_fixture(directory.path());
        let mut loaded = engine.campaign().clone();
        let append_attempt = |campaign: &mut Campaign| {
            campaign.record_mission_attempt(
                0,
                robin_engine::campaign_history::MissionAttemptOutcome::Won,
                Some(100),
                Some(0xbeef),
                70,
                engine_api::SimConfig::default(),
                &robin_engine::mission_stat::MissionStat::default(),
                None,
            );
        };
        append_attempt(&mut loaded);
        // This is the callback used by Game::handle_quit; it must import
        // pre-existing attempts even though no new terminal command runs.
        let target = directory.path().join("profiles.json");
        std::fs::create_dir(&target).unwrap();
        crate::game::GameCallbacks::synchronize_profile_with_campaign(
            &mut callbacks,
            &loaded,
            &profiles,
        );
        assert_eq!(
            callbacks
                .application_context
                .with_player_profiles_mut(|manager| manager
                    .get_active_mut()
                    .unwrap()
                    .lifetime_campaign_totals()
                    .attempts)
                .unwrap(),
            1,
        );
        std::fs::remove_dir(&target).unwrap();
        crate::game::GameCallbacks::synchronize_profile_with_campaign(
            &mut callbacks,
            &loaded,
            &profiles,
        );
        let store = crate::player_profile_store::PlayerProfileStore::for_directory(
            directory.path().to_str().unwrap(),
        );
        assert_eq!(
            store.load().unwrap().profiles[0]
                .lifetime_campaign_totals()
                .attempts,
            1
        );
        assert_eq!(
            loaded.latest_mission_attempt().unwrap().sequence(),
            1,
            "synchronization never invents the pending terminal attempt"
        );
        append_attempt(&mut loaded);
        RustCallbacks::promote_terminal_profile(&callbacks.application_context, &loaded, &profiles);
        RustCallbacks::promote_terminal_profile(&callbacks.application_context, &loaded, &profiles);
        assert_eq!(
            store.load().unwrap().profiles[0]
                .lifetime_campaign_totals()
                .attempts,
            2
        );
    }

    /// Regression for crash fingerprint 0279c47b1873225f (issue #74): a
    /// lifetime attempt whose durable `(campaign_run_id, sequence)` key is
    /// already occupied by a different canonical mission must be logged, not
    /// panicked on, at terminal debriefing. The original replay bundle is not
    /// part of this checkout, so the conflicting state is built deterministically.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn conflicting_lifetime_attempt_does_not_panic_at_terminal_promotion() {
        const RUN_ID: u64 = 0xDEAD_BEEF;
        const CANONICAL_MISSION_ID: u32 = 17;
        const CONFLICTING_MISSION_ID: u32 = 0xC0FFEE;

        let directory = tempfile::tempdir().unwrap();
        let (callbacks, _host, engine, _assets, _game, profiles) =
            super::super::operation_outcome_tests::diagnostic_callback_fixture(directory.path());
        let application = &callbacks.application_context;

        // Seed attempt 5 of one canonical run against mission 17.
        let mut seed = engine.campaign().clone();
        for _ in 0..5 {
            seed.record_mission_attempt(
                0,
                robin_engine::campaign_history::MissionAttemptOutcome::Won,
                Some(100),
                Some(RUN_ID),
                70,
                engine_api::SimConfig::default(),
                &robin_engine::mission_stat::MissionStat::default(),
                None,
            );
        }
        RustCallbacks::promote_terminal_profile(application, &seed, &profiles);
        assert_eq!(
            application
                .with_player_profiles(|manager| manager
                    .get_active()
                    .unwrap()
                    .lifetime_campaign_totals()
                    .attempts)
                .unwrap(),
            5
        );

        // The same run's attempt 5 now belongs to a different mission, so
        // promote_campaign raises ConflictingAttempt on the durable key.
        let mut conflicting_profiles = profiles.clone();
        conflicting_profiles
            .missions
            .push(robin_engine::profiles::MissionProfile {
                id: CONFLICTING_MISSION_ID,
                mission_name: "Conflicting Mission".into(),
                ..Default::default()
            });
        let mut conflicting_campaign = engine.campaign().clone();
        conflicting_campaign
            .missions
            .push(robin_engine::mission::Mission {
                profile_idx: Some(1),
                ..Default::default()
            });
        for _ in 0..4 {
            conflicting_campaign.record_mission_attempt(
                0,
                robin_engine::campaign_history::MissionAttemptOutcome::Won,
                Some(100),
                Some(RUN_ID),
                70,
                engine_api::SimConfig::default(),
                &robin_engine::mission_stat::MissionStat::default(),
                None,
            );
        }
        conflicting_campaign.record_mission_attempt(
            1,
            robin_engine::campaign_history::MissionAttemptOutcome::Lost,
            Some(100),
            Some(RUN_ID),
            70,
            engine_api::SimConfig::default(),
            &robin_engine::mission_stat::MissionStat::default(),
            None,
        );

        // This used to abort the process through unwrap_or_else in
        // promote_recorded_history, reached from terminal debriefing.
        RustCallbacks::promote_terminal_profile(
            application,
            &conflicting_campaign,
            &conflicting_profiles,
        );

        let attempts = application
            .with_player_profiles(|manager| {
                manager
                    .get_active()
                    .unwrap()
                    .campaign_history
                    .attempts()
                    .iter()
                    .map(|entry| (entry.mission_id(), entry.attempt().sequence()))
                    .collect::<Vec<_>>()
            })
            .unwrap();
        assert_eq!(
            attempts.len(),
            5,
            "conflicting promotion must neither drop nor duplicate lifetime history"
        );
        assert!(
            attempts
                .iter()
                .all(|&(mission_id, _)| mission_id == CANONICAL_MISSION_ID),
            "conflicting canonical record must not overwrite the retained lifetime attempt"
        );
        assert!(
            attempts
                .iter()
                .any(|&(mission_id, sequence)| mission_id == CANONICAL_MISSION_ID && sequence == 5)
        );

        // Retry on the next save with the compatible campaign: still coherent.
        RustCallbacks::promote_terminal_profile(application, &seed, &profiles);
        assert_eq!(
            application
                .with_player_profiles(|manager| manager
                    .get_active()
                    .unwrap()
                    .campaign_history
                    .attempts()
                    .len())
                .unwrap(),
            5
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn failed_persistence_retry_writes_applied_metrics_without_recrediting() {
        let directory = tempfile::tempdir().unwrap();
        let (mut callbacks, _host, engine, _assets, _game, profiles) =
            super::super::operation_outcome_tests::diagnostic_callback_fixture(directory.path());
        let application = &callbacks.application_context;
        let campaign = engine.campaign();
        let mut credits = ProfileClockCredits::default();
        // A directory at the exact file target deterministically rejects the
        // atomic rename, without touching permissions or any player data.
        let target = directory.path().join("profiles.json");
        std::fs::create_dir(&target).unwrap();
        synchronize_metrics(application, &mut credits, campaign, &profiles, 60);
        assert_eq!(
            application
                .with_player_profiles_mut(|manager| manager.get_active_mut().unwrap().play_time)
                .unwrap(),
            60
        );
        std::fs::remove_dir(&target).unwrap();
        synchronize_metrics(application, &mut credits, campaign, &profiles, 60);
        let stored = crate::player_profile_store::PlayerProfileStore::for_directory(
            directory.path().to_str().unwrap(),
        )
        .load()
        .unwrap();
        assert_eq!(stored.profiles[0].play_time, 60);
        synchronize_metrics(application, &mut credits, campaign, &profiles, 70);
        let stored = crate::player_profile_store::PlayerProfileStore::for_directory(
            directory.path().to_str().unwrap(),
        )
        .load()
        .unwrap();
        assert_eq!(stored.profiles[0].play_time, 70);

        let mut completed = campaign.clone();
        completed.record_mission_attempt(
            0,
            robin_engine::campaign_history::MissionAttemptOutcome::Won,
            Some(100),
            Some(0xbeef),
            70,
            engine_api::SimConfig::default(),
            &robin_engine::mission_stat::MissionStat::default(),
            None,
        );
        RustCallbacks::promote_terminal_profile(application, &completed, &profiles);
        RustCallbacks::promote_terminal_profile(application, &completed, &profiles);
        let stored = crate::player_profile_store::PlayerProfileStore::for_directory(
            directory.path().to_str().unwrap(),
        )
        .load()
        .unwrap();
        assert_eq!(
            stored.profiles[0].play_time, 70,
            "history promotion does not credit time"
        );
        assert_eq!(stored.profiles[0].lifetime_campaign_totals().attempts, 1);
        synchronize_metrics(application, &mut credits, &completed, &profiles, 70);
        assert_eq!(
            application
                .with_player_profiles_mut(|manager| manager.get_active_mut().unwrap().play_time)
                .unwrap(),
            70,
            "post-terminal synchronization is still the same clock epoch"
        );
        callbacks.profile_clock_credits = credits;
        callbacks.begin_profile_clock_session();
        assert_eq!(
            callbacks
                .profile_clock_credits
                .credit(stored.profiles[0].id, 17, 20),
            20,
            "fresh mission bootstrap opens another epoch"
        );
    }
}

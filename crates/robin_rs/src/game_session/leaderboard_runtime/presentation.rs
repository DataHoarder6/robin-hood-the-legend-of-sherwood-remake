//! Cooperative post-mission UI ownership. Preparation and upload tasks are separate owners.

use super::*;
use crate::ingame_menu::widget_bridge::ModalScreenIo;

/// Cooperative presentation/background state installed only after the final
/// authoritative debrief decision has been recorded. The first poll therefore
/// happens one outer frame after that decision and cannot export a replay that
/// is missing its terminal record.
pub(in crate::game_session) struct MissionEndLeaderboardTaskState {
    phase: MissionEndLeaderboardTaskPhase,
    application_context: crate::host::ApplicationContext,
}

enum MissionEndLeaderboardTaskPhase {
    Preparing(MissionEndPreparation),
    Visible(MissionEndLeaderboardScreen),
    Unavailable(crate::save_recovery::ErrorNotice),
    Finished,
}

pub(in crate::game_session) enum MissionEndLeaderboardTaskProgress {
    Pending,
    Finished,
    Detach(MissionEndLeaderboardController),
}

impl MissionEndLeaderboardTaskState {
    pub(in crate::game_session) fn new(
        preparation: MissionEndPreparation,
        application_context: crate::host::ApplicationContext,
    ) -> Self {
        Self {
            phase: MissionEndLeaderboardTaskPhase::Preparing(preparation),
            application_context,
        }
    }

    pub(in crate::game_session) fn owns_presentation(&self) -> bool {
        matches!(
            &self.phase,
            MissionEndLeaderboardTaskPhase::Preparing(_)
                | MissionEndLeaderboardTaskPhase::Visible(_)
                | MissionEndLeaderboardTaskPhase::Unavailable(_)
        )
    }

    /// Settle a preparation result that cannot open a board or a notice.
    /// `None` leaves the normal board/notice handling in charge. Both `tick`
    /// and the no-replay test go through this method, so the phase transition
    /// cannot drift from the decision it reports. Renderer-free.
    pub(in crate::game_session) fn settle_preparation(
        &mut self,
        result: &Option<Result<(MissionEndRunBundle, Arc<[u8]>), RankedError>>,
    ) -> Option<MissionEndLeaderboardTaskProgress> {
        let progress = match result {
            Some(Err(RankedError::NoReplay(_))) => {
                tracing::debug!(
                    "mission-end leaderboards skipped: no replay snapshot bytes are available"
                );
                MissionEndLeaderboardTaskProgress::Finished
            }
            _ => return None,
        };
        self.phase = MissionEndLeaderboardTaskPhase::Finished;
        Some(progress)
    }

    /// Poll preparation and apply its result. Renderer-free until the board
    /// becomes visible or the preparing page needs drawing.
    fn tick_preparation(
        &mut self,
        io: &mut ModalScreenIo<'_, '_>,
    ) -> MissionEndLeaderboardTaskProgress {
        let (preferences, result) = {
            let MissionEndLeaderboardTaskPhase::Preparing(preparation) = &mut self.phase else {
                unreachable!("tick_preparation is only called in the preparing phase")
            };
            let preferences = preparation.preferences().clone();
            let replay_exports = self.application_context.replay_exports();
            let edition = super::installed_content_edition(&self.application_context);
            (
                preferences,
                preparation.poll_bundle(&replay_exports, edition),
            )
        };
        if let Some(progress) = self.settle_preparation(&result) {
            return progress;
        }
        let Some(result) = result else {
            render_preparing(&mut *io.renderer, io.resources, io.cursor);
            return MissionEndLeaderboardTaskProgress::Pending;
        };
        let (bundle, bytes) = match result {
            Ok(bundle) => bundle,
            Err(error) => {
                tracing::warn!("mission-end leaderboards unavailable: {error}");
                if !preferences.show_mission_end_boards {
                    self.phase = MissionEndLeaderboardTaskPhase::Finished;
                    return MissionEndLeaderboardTaskProgress::Finished;
                }
                self.phase = MissionEndLeaderboardTaskPhase::Unavailable(
                    crate::save_recovery::ErrorNotice::new(format!(
                        "Leaderboards unavailable: {error}"
                    )),
                );
                return MissionEndLeaderboardTaskProgress::Pending;
            }
        };
        let api = match LeaderboardApi::from_preferences(&preferences) {
            Ok(api) => api,
            Err(error) => {
                tracing::warn!("mission-end leaderboard endpoint unavailable: {error}");
                self.phase = MissionEndLeaderboardTaskPhase::Finished;
                return MissionEndLeaderboardTaskProgress::Finished;
            }
        };
        let controller = MissionEndLeaderboardController::new(
            bundle,
            bytes,
            preferences,
            Box::new(HttpMissionEndLeaderboardBackend::new(api)),
        );
        let mut controller = match controller {
            Ok(controller) => controller,
            Err(error) => {
                tracing::warn!("mission-end leaderboard setup failed: {error}");
                self.phase = MissionEndLeaderboardTaskPhase::Finished;
                return MissionEndLeaderboardTaskProgress::Finished;
            }
        };
        controller.enable_history_tracking();
        if controller.is_visible() {
            self.phase = MissionEndLeaderboardTaskPhase::Visible(MissionEndLeaderboardScreen::new(
                controller,
                io.resources,
            ));
            // Do not poll the freshly-created controller a second time
            // in this host frame.
            MissionEndLeaderboardTaskProgress::Pending
        } else {
            controller
                .apply_action(MissionEndLeaderboardAction::Close)
                .unwrap_or_else(|error| {
                    panic!("validated hidden leaderboard could not close: {error}")
                });
            self.phase = MissionEndLeaderboardTaskPhase::Finished;
            retire_or_detach(controller)
        }
    }

    pub(in crate::game_session) fn tick(
        &mut self,
        io: &mut ModalScreenIo<'_, '_>,
    ) -> MissionEndLeaderboardTaskProgress {
        if matches!(self.phase, MissionEndLeaderboardTaskPhase::Preparing(_)) {
            return self.tick_preparation(io);
        }
        let window = &mut *io.window;
        let renderer = &mut *io.renderer;
        let resources = io.resources;
        let cursor = io.cursor;
        match &mut self.phase {
            MissionEndLeaderboardTaskPhase::Preparing(_) => {
                unreachable!("the preparing phase is handled before borrowing modal IO")
            }
            MissionEndLeaderboardTaskPhase::Visible(screen) => {
                let event = screen.tick(&mut ModalScreenIo {
                    window,
                    renderer,
                    resources,
                    cursor,
                });
                if let Err(error) = screen
                    .controller_mut()
                    .persist_queued_receipt_watch(&self.application_context)
                {
                    tracing::error!(
                        "queued leaderboard verification could not be handed to durable tracking: {error}"
                    );
                }
                if event != Some(MissionEndLeaderboardEvent::Closed) {
                    return MissionEndLeaderboardTaskProgress::Pending;
                }
                let MissionEndLeaderboardTaskPhase::Visible(screen) =
                    std::mem::replace(&mut self.phase, MissionEndLeaderboardTaskPhase::Finished)
                else {
                    unreachable!()
                };
                let controller = screen.into_controller();
                retire_or_detach(controller)
            }
            MissionEndLeaderboardTaskPhase::Unavailable(notice) => {
                if notice.tick(&mut ModalScreenIo {
                    window,
                    renderer,
                    resources,
                    cursor,
                }) {
                    self.phase = MissionEndLeaderboardTaskPhase::Finished;
                    MissionEndLeaderboardTaskProgress::Finished
                } else {
                    MissionEndLeaderboardTaskProgress::Pending
                }
            }
            MissionEndLeaderboardTaskPhase::Finished => MissionEndLeaderboardTaskProgress::Finished,
        }
    }
}

fn retire_or_detach(
    controller: MissionEndLeaderboardController,
) -> MissionEndLeaderboardTaskProgress {
    if controller.can_retire_after_close() {
        MissionEndLeaderboardTaskProgress::Finished
    } else {
        MissionEndLeaderboardTaskProgress::Detach(controller)
    }
}

fn render_preparing(
    renderer: &mut Renderer,
    resources: &IngameMenuResources,
    cursor: Option<&ModalCursor<'_>>,
) {
    enter_modal_gpu_phase(renderer);
    dim_screen(renderer);
    if let Some(background) = resources.menu_bg[0] {
        draw_screen_background(renderer, &background);
    }
    if let Some(font) = resources.title_font_any() {
        let transform = MenuTransform::centered(
            i32::from(renderer.screen_width()),
            i32::from(renderer.screen_height()),
        );
        let text = "Loading verified leaderboards...";
        render_text_virt_font(
            renderer,
            font,
            transform,
            text,
            (crate::ingame_menu::layout::MENU_W - font.text_width(text)) / 2,
            220,
        );
        if let Some(cursor) = cursor {
            // The preparation page has no controls, but retaining the current
            // cursor avoids a visible jump when the board becomes ready.
            cursor.draw(
                renderer,
                &crate::ingame_menu::widget_bridge::ModalInputState::default(),
            );
        }
    }
    renderer.present();
}

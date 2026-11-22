//! Pure chat-canvas lane geometry, ported from KSG Code's `chatCanvasLayout.ts` and
//! `threadDetailsCardLayout.ts` (`apps/web/src/components/chat`). The transcript lane, the
//! composer lane and the floating Thread Details card share this math: a card that fits
//! inline shifts the lane left by `DETAILS_CARD_CLEARANCE`; otherwise it falls back to a
//! popover. KSG's preview mini-player branches are not ported — Shina has no floating
//! player — so `resolve_chat_canvas_layout` keeps the centered-vs-shifted lane only, and the
//! card layout keeps the player's obstacle shape for a future port.

/// KSG's `DETAILS_CARD_CLEARANCE`: the gap the chat lane keeps from an inline Details card.
pub(crate) const DETAILS_CARD_CLEARANCE: f32 = 32.0;
/// KSG's `THREAD_DETAILS_CARD_GAP`: the card's inset from the container's right edge.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) const THREAD_DETAILS_CARD_GAP: f32 = 12.0;
/// KSG's `THREAD_DETAILS_CARD_WIDTH`: the docked card's fixed width. Popovers use
/// `--thread-details-panel-width`; the inline card is fixed.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) const THREAD_DETAILS_CARD_WIDTH: f32 = 272.0;
/// KSG's `THREAD_DETAILS_BOTTOM_GAP`: room the card leaves at the container's bottom.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) const THREAD_DETAILS_BOTTOM_GAP: f32 = 18.0;
/// KSG's `160` floor: under it the card opens as a popover instead.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) const MIN_DETAILS_CARD_HEIGHT: f32 = 160.0;
/// KSG's `padding = 48` lane padding default (`sm:ps-12`). Shina passes its responsive
/// `lane_gutter`, which is 12 below `sm` (640 px).
pub(crate) const LANE_PADDING: f32 = 48.0;
/// KSG's `maxChatWidth = 768` default (`--chat-content-max-width`). Shina passes `measure`.
pub(crate) const MAX_CHAT_WIDTH: f32 = 768.0;
/// KSG's `minChatWidth = 640` (`min-w-[40rem]` on the canvas probe).
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) const MIN_CHAT_WIDTH: f32 = 640.0;

/// The card as a `detailsCard` obstacle edge set in canvas coordinates, as KSG's
/// `PreviewMiniPlayerObstacles["detailsCard"]`.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct DetailsCardEdges {
    pub left: f32,
    pub right: f32,
    pub bottom: f32,
}

/// KSG's `PreviewMiniPlayerFrame` fields the card's overlap check reads.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct MiniPlayerFrame {
    pub x: f32,
    pub y: f32,
    pub width: f32,
}

/// The chat lane's resolved box: `left`/`width` are the content rect inside the container
/// and `inset_start`/`inset_end` are the extra per-side padding that produces it, matching
/// KSG's `--chat-lane-inset-start`/`--chat-lane-inset-end` on `.chat-composer-lane` and
/// `.messages-timeline-scroll`. `inset_start` is always 0 today, as in KSG.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ChatLane {
    pub left: f32,
    pub width: f32,
    pub inset_start: f32,
    pub inset_end: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ChatCanvasLayout {
    pub chat: ChatLane,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ChatCanvasInput {
    pub container_width: f32,
    /// KSG default `48`; Shina passes `lane_gutter` for the compact 12 px fallback.
    pub padding: f32,
    /// KSG default `768` (`--chat-content-max-width`); Shina passes `measure(wide)`.
    pub max_chat_width: f32,
    /// `left`/`right`/`bottom` edges of the inline Details card, when mounted.
    pub details_card: Option<DetailsCardEdges>,
}

impl Default for ChatCanvasInput {
    fn default() -> Self {
        Self {
            container_width: 0.0,
            padding: LANE_PADDING,
            max_chat_width: MAX_CHAT_WIDTH,
            details_card: None,
        }
    }
}

/// KSG's `resolveChatCanvasLayout` with the preview player dropped. The lane is centred in
/// the container at `max_chat_width` until it collides with the Details card; then its right
/// edge stops `DETAILS_CARD_CLEARANCE` short of the card and the difference lands in
/// `inset_end` so the caller can add it to the lane's right padding.
pub(crate) fn resolve_chat_canvas_layout(input: ChatCanvasInput) -> ChatCanvasLayout {
    let padding = input.padding;
    let centered_width = input
        .max_chat_width
        .min(input.container_width - padding * 2.0)
        .max(0.0);
    let lane_right = input
        .details_card
        .map_or(input.container_width - padding, |card| {
            card.left - DETAILS_CARD_CLEARANCE
        });
    let normal_width = centered_width.min(lane_right - padding).max(0.0);
    let canvas_center_left = (input.container_width - normal_width) / 2.0;
    let normal_left = padding.max(canvas_center_left.min(lane_right - normal_width));
    ChatCanvasLayout {
        chat: ChatLane {
            left: normal_left,
            width: normal_width,
            inset_start: 0.0,
            inset_end: (input.container_width - normal_left * 2.0 - normal_width).max(0.0),
        },
    }
}

/// KSG's `resolveThreadDetailsCardDensity`: how much of the card's content fits. `0` means
/// the density's height is unmeasured, which KSG treats as always fitting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) enum ThreadDetailsCardDensity {
    Full,
    Compact,
    Essential,
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn resolve_thread_details_card_density(
    height: f32,
    full: f32,
    compact: f32,
) -> ThreadDetailsCardDensity {
    if full == 0.0 || full <= height {
        ThreadDetailsCardDensity::Full
    } else if compact == 0.0 || compact <= height {
        ThreadDetailsCardDensity::Compact
    } else {
        ThreadDetailsCardDensity::Essential
    }
}

/// KSG's `resolveThreadDetailsCardLayout` return: `right` is the inset from the container's
/// right edge, kept as `right_inset` so callers can place it either way.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct DetailsCardRect {
    pub x: f32,
    pub right_inset: f32,
    pub width: f32,
    pub y: f32,
    pub height: f32,
}

impl DetailsCardRect {
    /// The card's obstacle edges for `resolve_chat_canvas_layout`, as KSG's
    /// `resolveChatCanvasModel` builds them from the placement.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn edges(&self) -> DetailsCardEdges {
        DetailsCardEdges {
            left: self.x,
            right: self.x + self.width,
            bottom: self.y + self.height,
        }
    }
}

#[derive(Clone, Copy, Debug)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct ThreadDetailsCardInput {
    pub container_width: f32,
    pub container_height: f32,
    /// While a panel animates the container, the card fits only if it fits at both ends.
    /// `None` means the container's current width, as in KSG.
    pub narrowest_width: Option<f32>,
    /// KSG's `lane.padding`.
    pub lane_padding: f32,
    /// KSG's `lane.minChatWidth`.
    pub lane_min_chat_width: f32,
    /// Shina's addition: with the docked right Inspector open the card uses its popover
    /// presentation instead of mounting inline.
    pub right_sidebar_open: bool,
    /// KSG's preview `frame` and `overlapsDetailsCard`: the player over the card's slot
    /// shortens the card to stay above it.
    pub frame: Option<MiniPlayerFrame>,
    pub overlaps_details_card: bool,
    /// KSG's `topInset`: a find bar's bottom edge pushes the card down.
    pub top_inset: f32,
}

impl Default for ThreadDetailsCardInput {
    #[cfg_attr(not(test), allow(dead_code))]
    fn default() -> Self {
        Self {
            container_width: 0.0,
            container_height: 0.0,
            narrowest_width: None,
            lane_padding: LANE_PADDING,
            lane_min_chat_width: MIN_CHAT_WIDTH,
            right_sidebar_open: false,
            frame: None,
            overlaps_details_card: false,
            top_inset: 0.0,
        }
    }
}

/// KSG's `resolveThreadDetailsCardLayout`: `Some` when the card can dock at the container's
/// right edge, `None` when it must fall back to the popover presentation.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn resolve_thread_details_card_layout(
    input: ThreadDetailsCardInput,
) -> Option<DetailsCardRect> {
    if input.right_sidebar_open {
        return None;
    }
    let gap = THREAD_DETAILS_CARD_GAP;
    let width = THREAD_DETAILS_CARD_WIDTH;
    let x = input.container_width - width - gap;
    // The transition ends are sampled once, so a resize mid-animation can leave them
    // wider than the container is now.
    let fit_width = input
        .narrowest_width
        .unwrap_or(input.container_width)
        .min(input.container_width);
    if fit_width - width - gap - DETAILS_CARD_CLEARANCE - input.lane_padding
        < input.lane_min_chat_width
    {
        return None;
    }
    let y = gap + input.top_inset;
    let height = match (input.overlaps_details_card, input.frame) {
        (true, Some(frame)) if frame.x + frame.width > x - gap && frame.x < x + width + gap => {
            (input.container_height - y - THREAD_DETAILS_BOTTOM_GAP).min(frame.y - y - gap)
        }
        _ => input.container_height - y - THREAD_DETAILS_BOTTOM_GAP,
    };
    if height < MIN_DETAILS_CARD_HEIGHT {
        return None;
    }
    Some(DetailsCardRect {
        x,
        right_inset: gap,
        width,
        y,
        height,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lane(container_width: f32) -> ChatLane {
        resolve_chat_canvas_layout(ChatCanvasInput {
            container_width,
            ..Default::default()
        })
        .chat
    }

    #[test]
    fn wide_containers_get_the_centered_lane() {
        let lane = lane(1440.0);
        assert_eq!(lane.width, 768.0);
        assert_eq!(lane.left, (1440.0 - 768.0) / 2.0);
        assert_eq!(lane.inset_start, 0.0);
        assert_eq!(lane.inset_end, 0.0);
    }

    #[test]
    fn narrow_containers_fill_between_the_gutters() {
        let lane = lane(700.0);
        assert_eq!(lane.width, 700.0 - 2.0 * 48.0);
        assert_eq!(lane.left, 48.0);
        assert_eq!(lane.inset_end, 0.0);
    }

    #[test]
    fn an_inline_details_card_shifts_the_lane_left_with_clearance() {
        // Card at x = 1200 - 272 - 12 = 916, as `resolve_thread_details_card_layout` puts it.
        let card = resolve_thread_details_card_layout(ThreadDetailsCardInput {
            container_width: 1200.0,
            container_height: 800.0,
            ..Default::default()
        })
        .unwrap()
        .edges();
        assert_eq!(card.left, 916.0);
        let lane = resolve_chat_canvas_layout(ChatCanvasInput {
            container_width: 1200.0,
            details_card: Some(card),
            ..Default::default()
        })
        .chat;
        assert_eq!(lane.width, 768.0);
        // laneRight = 916 - 32 = 884; centre (216) clamps to 884 - 768 = 116.
        assert_eq!(lane.left, 116.0);
        assert_eq!(lane.left + lane.width, 916.0 - DETAILS_CARD_CLEARANCE);
        assert_eq!(lane.inset_end, 1200.0 - 2.0 * 116.0 - 768.0);
        // A card far enough right leaves the centred lane alone.
        let far = resolve_chat_canvas_layout(ChatCanvasInput {
            container_width: 1400.0,
            details_card: Some(DetailsCardEdges {
                left: 1116.0,
                right: 1388.0,
                bottom: 200.0,
            }),
            ..Default::default()
        })
        .chat;
        assert_eq!(far.left, (1400.0 - 768.0) / 2.0);
        assert_eq!(far.inset_end, 0.0);
    }

    #[test]
    fn the_card_falls_back_to_a_popover_when_the_lane_would_shrink() {
        // 640 + 48 + 32 + 272 + 12 = 1004: at 1003 the lane drops under `minChatWidth`.
        let narrow = resolve_thread_details_card_layout(ThreadDetailsCardInput {
            container_width: 1003.0,
            container_height: 800.0,
            ..Default::default()
        });
        assert!(narrow.is_none());
        let fits = resolve_thread_details_card_layout(ThreadDetailsCardInput {
            container_width: 1004.0,
            container_height: 800.0,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(
            fits.x,
            1004.0 - THREAD_DETAILS_CARD_WIDTH - THREAD_DETAILS_CARD_GAP
        );
        assert_eq!(fits.y, THREAD_DETAILS_CARD_GAP);
        assert_eq!(fits.height, 800.0 - fits.y - THREAD_DETAILS_BOTTOM_GAP);
        // A narrower transition sample counts too.
        let animating = resolve_thread_details_card_layout(ThreadDetailsCardInput {
            container_width: 1200.0,
            container_height: 800.0,
            narrowest_width: Some(900.0),
            ..Default::default()
        });
        assert!(animating.is_none());
    }

    #[test]
    fn the_card_falls_back_on_short_containers() {
        // 12 + 160 + 18 = 190: at 189 px tall the card cannot reach its minimum height.
        let short = resolve_thread_details_card_layout(ThreadDetailsCardInput {
            container_width: 1200.0,
            container_height: 189.0,
            ..Default::default()
        });
        assert!(short.is_none());
        let tall = resolve_thread_details_card_layout(ThreadDetailsCardInput {
            container_width: 1200.0,
            container_height: 190.0,
            ..Default::default()
        });
        assert_eq!(tall.unwrap().height, MIN_DETAILS_CARD_HEIGHT);
    }

    #[test]
    fn a_docked_right_sidebar_forces_the_popover() {
        let card = resolve_thread_details_card_layout(ThreadDetailsCardInput {
            container_width: 1920.0,
            container_height: 1080.0,
            right_sidebar_open: true,
            ..Default::default()
        });
        assert!(card.is_none());
    }

    #[test]
    fn an_overlapping_player_shortens_the_card() {
        let frame = MiniPlayerFrame {
            x: 1600.0,
            y: 500.0,
            width: 300.0,
        };
        let card = resolve_thread_details_card_layout(ThreadDetailsCardInput {
            container_width: 1920.0,
            container_height: 1080.0,
            frame: Some(frame),
            overlaps_details_card: true,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(card.height, frame.y - card.y - THREAD_DETAILS_CARD_GAP);
    }

    #[test]
    fn density_picks_the_richest_tier_that_fits() {
        assert_eq!(
            resolve_thread_details_card_density(800.0, 700.0, 400.0),
            ThreadDetailsCardDensity::Full
        );
        assert_eq!(
            resolve_thread_details_card_density(500.0, 700.0, 400.0),
            ThreadDetailsCardDensity::Compact
        );
        assert_eq!(
            resolve_thread_details_card_density(300.0, 700.0, 400.0),
            ThreadDetailsCardDensity::Essential
        );
        // Unmeasured heights (0) always fit.
        assert_eq!(
            resolve_thread_details_card_density(100.0, 0.0, 400.0),
            ThreadDetailsCardDensity::Full
        );
        assert_eq!(
            resolve_thread_details_card_density(100.0, 700.0, 0.0),
            ThreadDetailsCardDensity::Compact
        );
    }
}

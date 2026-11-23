use super::*;

/// A model's display name and its CPA route: "Gemini 3.8 Flash (CPA → Antigravity)" is
/// ("Gemini 3.8 Flash", Some("CPA → Antigravity")). Names without a route stay whole.
pub(super) fn split_model_label(label: &str) -> (&str, Option<&str>) {
    let label = label.trim();
    if let Some(open) = label.rfind(" (")
        && label.ends_with(')')
    {
        return (&label[..open], Some(&label[open + 2..label.len() - 1]));
    }
    (label, None)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Vendor {
    OpenAi,
    Anthropic,
    Antigravity,
    Other,
}

/// The vendor behind a CPA model, from its route first and its id second, so the icon follows
/// KSG Code's provider marks: Codex is OpenAI, Claude is Anthropic, Antigravity is Google's.
pub(super) fn model_vendor(id: &str, label: &str) -> Vendor {
    let (_, route) = split_model_label(label);
    let route = route.unwrap_or("").to_lowercase();
    let id = id.to_lowercase();
    let either = |words: &[&str]| words.iter().any(|w| route.contains(w) || id.contains(w));
    if either(&["codex", "openai"]) || id.starts_with("gpt") || id.starts_with("o3") {
        Vendor::OpenAi
    } else if either(&["anthropic", "claude"]) {
        Vendor::Anthropic
    } else if either(&["antigravity", "gemini", "google"]) {
        Vendor::Antigravity
    } else {
        Vendor::Other
    }
}

pub(super) static OPENAI_ICON: Icon = Icon {
    name: "OPENAI",
    view: 411.0,
    body: r#"<g transform="translate(-100 -100)"><path fill="currentColor" fill-rule="evenodd" clip-rule="evenodd" d="M252.794 108.802C289.191 99.0484 326.265 110.305 351.148 135.135C385.113 126.072 422.85 134.862 449.492 161.505C476.136 188.149 484.925 225.888 475.862 259.85V259.854C500.696 284.735 511.95 321.81 502.198 358.207C492.447 394.602 464.161 421.084 430.215 430.217C421.083 464.162 394.603 492.448 358.206 502.199C321.812 511.951 284.734 500.693 259.852 475.864C225.887 484.927 188.15 476.137 161.507 449.495C134.864 422.851 126.073 385.111 135.136 351.149C110.304 326.266 99.0496 289.192 108.801 252.795C118.552 216.4 146.84 189.918 180.784 180.785C189.917 146.841 216.396 118.553 252.794 108.802ZM374.292 407.145C374.292 411.271 372.092 415.086 368.517 417.148L283.723 466.102C302.487 480.585 327.555 486.459 352.217 479.852C386.997 470.532 410.068 439.312 410.555 405.006V317.717C410.555 315.08 409.125 312.621 406.843 311.303L374.292 292.509V407.145ZM251.868 415.897C248.296 417.959 243.893 417.959 240.317 415.897L155.526 366.942C152.366 390.436 159.811 415.08 177.866 433.136H177.863C203.325 458.594 241.896 462.962 271.85 446.232L347.449 402.586C349.735 401.268 351.148 398.8 351.148 396.163V358.579L251.868 415.897ZM368.602 220.628C366.319 219.309 363.474 219.318 361.191 220.637L328.641 239.431L427.921 296.749C431.496 298.811 433.697 302.627 433.697 306.752V404.661C455.622 395.654 473.244 376.881 479.851 352.218C489.169 317.442 473.668 281.85 444.201 264.274L368.602 220.628ZM177.303 206.34C155.377 215.348 137.756 234.122 131.148 258.783C121.832 293.561 137.331 329.153 166.799 346.727L242.398 390.373C244.68 391.692 247.525 391.684 249.807 390.366L282.357 371.572L183.078 314.253C179.504 312.189 177.303 308.375 177.303 304.251V206.34ZM259.849 279.145V331.858L305.5 358.213L351.15 331.858V279.145L305.5 252.789L259.849 279.145ZM327.276 144.9C308.512 130.418 283.445 124.543 258.782 131.15C224.002 140.471 200.931 171.691 200.445 205.995V293.286C200.445 295.923 201.875 298.381 204.158 299.7L236.707 318.493V203.856C236.707 199.731 238.909 195.916 242.483 193.853L327.276 144.9ZM433.137 177.867C407.675 152.407 369.103 148.038 339.149 164.769L263.55 208.415C261.265 209.734 259.852 212.202 259.852 214.838V252.423L359.132 195.105C362.703 193.041 367.108 193.041 370.682 195.105L455.473 244.06C458.635 220.567 451.189 195.922 433.135 177.867H433.137Z"/></g>"#,
};

pub(super) static CLAUDE_ICON: Icon = Icon {
    name: "CLAUDE",
    view: 257.0,
    body: r##"<path fill="#d97757" d="m50.228 170.321 50.357-28.257.843-2.463-.843-1.361h-2.462l-8.426-.518-28.775-.778-24.952-1.037-24.175-1.296-6.092-1.297L0 125.796l.583-3.759 5.12-3.434 7.324.648 16.202 1.101 24.304 1.685 17.629 1.037 26.118 2.722h4.148l.583-1.685-1.426-1.037-1.101-1.037-25.147-17.045-27.22-18.017-14.258-10.37-7.713-5.25-3.888-4.925-1.685-10.758 7-7.713 9.397.649 2.398.648 9.527 7.323 20.35 15.75L94.817 91.9l3.889 3.24 1.555-1.102.195-.777-1.75-2.917-14.453-26.118-15.425-26.572-6.87-11.018-1.814-6.61c-.648-2.723-1.102-4.991-1.102-7.778l7.972-10.823L71.42 0 82.05 1.426l4.472 3.888 6.61 15.101 10.694 23.786 16.591 32.34 4.861 9.592 2.592 8.879.973 2.722h1.685v-1.556l1.36-18.211 2.528-22.36 2.463-28.776.843-8.1 4.018-9.722 7.971-5.25 6.222 2.981 5.12 7.324-.713 4.73-3.046 19.768-5.962 30.98-3.889 20.739h2.268l2.593-2.593 10.499-13.934 17.628-22.036 7.778-8.749 9.073-9.657 5.833-4.601h11.018l8.1 12.055-3.628 12.443-11.342 14.388-9.398 12.184-13.48 18.147-8.426 14.518.778 1.166 2.01-.194 30.46-6.481 16.462-2.982 19.637-3.37 8.88 4.148.971 4.213-3.5 8.62-20.998 5.184-24.628 4.926-36.682 8.685-.454.324.519.648 16.526 1.555 7.065.389h17.304l32.21 2.398 8.426 5.574 5.055 6.805-.843 5.184-12.962 6.611-17.498-4.148-40.83-9.721-14-3.5h-1.944v1.167l11.666 11.406 21.387 19.314 26.767 24.887 1.36 6.157-3.434 4.86-3.63-.518-23.526-17.693-9.073-7.972-20.545-17.304h-1.36v1.814l4.73 6.935 25.017 37.59 1.296 11.536-1.814 3.76-6.481 2.268-7.13-1.297-14.647-20.544-15.1-23.138-12.185-20.739-1.49.843-7.194 77.448-3.37 3.953-7.778 2.981-6.48-4.925-3.436-7.972 3.435-15.749 4.148-20.544 3.37-16.333 3.046-20.285 1.815-6.74-.13-.454-1.49.194-15.295 20.999-23.267 31.433-18.406 19.702-4.407 1.75-7.648-3.954.713-7.064 4.277-6.286 25.47-32.405 15.36-20.092 9.917-11.6-.065-1.686h-.583L44.07 198.125l-12.055 1.555-5.185-4.86.648-7.972 2.463-2.593 20.35-13.999-.064.065Z"/>"##,
};

pub(super) fn provider_glyph(vendor: Vendor) -> &'static Icon {
    match vendor {
        Vendor::OpenAi => &OPENAI_ICON,
        Vendor::Anthropic => &CLAUDE_ICON,
        Vendor::Antigravity | Vendor::Other => &icons::CHIP,
    }
}

/// Provider marks ported from KSG Code's `Icons.tsx` (`OpenAI`, `ClaudeAI`, `AntigravityIcon`).
pub(super) fn provider_icon(s: &State, p: Palette, vendor: Vendor, size: f32) -> El {
    match vendor {
        Vendor::Antigravity => el()
            .size(size, size)
            .pixels_fit(s.artwork.antigravity.pixels(), rune::SurfaceFit::Fit)
            .a11y_hidden(),
        Vendor::OpenAi => rune::icon(&OPENAI_ICON, size).color(p.ink).a11y_hidden(),
        Vendor::Anthropic => rune::icon(&CLAUDE_ICON, size).a11y_hidden(),
        Vendor::Other => rune::icon(&icons::CHIP, size).color(p.muted).a11y_hidden(),
    }
}

pub(super) fn crewmate_avatar(
    s: &State,
    p: Palette,
    model_or_provider: &str,
    status: Status,
    size: f32,
) -> El {
    mascot_avatar(
        s,
        p,
        crate::visuals::provider_mascot_for(model_or_provider),
        status,
        size,
    )
}

pub(super) fn mascot_avatar(
    s: &State,
    p: Palette,
    mascot: crate::visuals::ProviderMascot,
    status: Status,
    size: f32,
) -> El {
    use crate::visuals::ProviderMascot::*;
    let pixels = s
        .artwork
        .mascot_pixels(mascot, size)
        .or_else(|| s.artwork.mascot_pixels(Default, size));
    let image = if let Some(pixels) = pixels {
        portrait(&pixels, size, p, None)
    } else {
        let (vendor, tint) = match mascot {
            OpenAi => (Vendor::OpenAi, p.ok_bg),
            Anthropic => (Vendor::Anthropic, p.brass_bg),
            Google => (Vendor::Antigravity, p.sea_bg),
            DeepSeek | Glm => (Vendor::Other, p.sea_bg),
            Qwen | Grok | Default => (Vendor::Other, p.lift),
        };
        el().size(size, size)
            .radius(portrait_radius(size))
            .fill(tint)
            .border(1.0, p.portrait_frame)
            .center()
            .child(provider_icon(s, p, vendor, (size * 0.6).min(20.0)))
    };
    haloed(s, p, image, size, status).a11y_hidden()
}

/// Pi's level ids in the slider's words: "xhigh" reads "XHigh", "medium" reads "Medium".
pub(super) fn effort_name(level: &str) -> String {
    match level {
        "" => "Default".into(),
        "xhigh" => "XHigh".into(),
        _ => {
            let mut chars = level.chars();
            chars
                .next()
                .map(|c| c.to_uppercase().collect::<String>() + chars.as_str())
                .unwrap_or_default()
        }
    }
}

/// Shared by the visible footer, its measurement probe and any open picker in this view.
pub(super) struct ComposerModelDisplay {
    pub(super) descriptors: Vec<crate::descriptors::ProviderOptionDescriptor>,
    pub(super) effort: String,
    pub(super) both_default: bool,
    pub(super) traits: crate::descriptors::TraitsTriggerDisplay,
    pub(super) selections: Vec<crate::model_selection::ModelSelection>,
    pub(super) label: String,
    pub(super) trigger_label: String,
    pub(super) unavailable: bool,
    pub(super) multi_labels: Vec<(String, String)>,
}

impl ComposerModelDisplay {
    pub(super) fn new(s: &State) -> Self {
        use crate::model_picker::get_display_model_name;
        let entries = s.provider_instance_entries();
        let (instance_id, model, options) = s.resolve_composer_model(&entries);
        let descriptors = s.model_descriptors(&instance_id, &model, &options);
        let selected = crate::model_selection::ModelSelection {
            instance_id,
            model,
            options,
        };
        let (effort, both_default, traits) = composer_effort(s, &descriptors, &selected);
        let entry = |id: &str| entries.iter().find(|e| e.instance_id == id);
        let selected_entry = entry(&selected.instance_id);
        let option = selected_entry.and_then(|e| s.picker_model_option(e, &selected.model));
        let display_name = |opt: &crate::model_picker::PickerModelOption, short| {
            get_display_model_name(
                &opt.name,
                opt.short_name.as_deref(),
                opt.sub_provider.as_deref(),
                short,
            )
        };
        let fallback = split_model_label(&s.current().model).0;
        let label = option
            .as_ref()
            .map(|opt| display_name(opt, true))
            .unwrap_or_else(|| fallback.to_owned());
        let trigger_label = if option.is_some() {
            label.clone()
        } else {
            crate::model_picker::retained_model_while_provider_locked(
                selected_entry,
                &[],
                &selected.model,
            )
            .map(|opt| display_name(&opt, true))
            .unwrap_or_else(|| label.clone())
        };
        let unavailable = option.as_ref().is_some_and(|opt| opt.is_unavailable);
        let selections = s.multi_model_selections_from(selected);
        let multi_labels = if selections.len() > 1 {
            selections
                .iter()
                .enumerate()
                .map(|(index, selection)| {
                    let other;
                    let opt = if index == 0 {
                        option.as_ref()
                    } else {
                        other = entry(&selection.instance_id)
                            .and_then(|e| s.picker_model_option(e, &selection.model));
                        other.as_ref()
                    };
                    let suffix = if opt.is_some_and(|m| m.is_unavailable) {
                        " (Unavailable)"
                    } else {
                        ""
                    };
                    let short = opt
                        .map(|m| display_name(m, true))
                        .unwrap_or_else(|| split_model_label(&selection.model).0.to_owned());
                    let full = opt
                        .map(|m| display_name(m, false))
                        .unwrap_or_else(|| selection.model.clone());
                    (format!("{short}{suffix}"), format!("{full}{suffix}"))
                })
                .collect()
        } else {
            Vec::new()
        };
        Self {
            descriptors,
            effort,
            both_default,
            traits,
            selections,
            label,
            trigger_label,
            unavailable,
            multi_labels,
        }
    }

    pub(super) fn show_effort(&self) -> bool {
        !self.both_default && !self.effort.is_empty() && self.selections.len() == 1
    }
}

/// The reasoning effort the composer shows, as KSG's `ComposerModelEffort` trigger reads it,
/// with the descriptors' trigger display. `None` when both the model and effort are defaults.
pub(super) fn composer_effort(
    s: &State,
    descriptors: &[crate::descriptors::ProviderOptionDescriptor],
    selection: &crate::model_selection::ModelSelection,
) -> (String, bool, crate::descriptors::TraitsTriggerDisplay) {
    let t = s.current();
    let (name, _) = split_model_label(&t.model);
    let controls = crate::descriptors::split_column_and_footer_controls(descriptors);
    let prompt_ultrathink = crate::descriptors::read_prompt_ultrathink(&t.draft.text);
    let primary_is_prompt_controlled = descriptors
        .iter()
        .find(|d| {
            matches!(
                d,
                crate::descriptors::ProviderOptionDescriptor::Select { .. }
            )
        })
        .is_some_and(|d| match d {
            crate::descriptors::ProviderOptionDescriptor::Select {
                prompt_injected_values,
                ..
            } => !prompt_injected_values.is_empty(),
            _ => false,
        })
        && prompt_ultrathink != crate::descriptors::PromptUltrathink::None;
    let display = crate::descriptors::build_traits_trigger_display(
        descriptors,
        primary_is_prompt_controlled,
        Some(selection),
        None,
    );
    let fallback_thinking = selection
        .options
        .iter()
        .find(|o| matches!(o.id.as_str(), "thinking" | "effort" | "reasoningEffort"))
        .and_then(|o| o.value.as_str())
        .unwrap_or(&t.thinking);
    let effort = if !display.label.is_empty() && controls.effort.is_some() {
        display
            .effort_label
            .clone()
            .unwrap_or_else(|| effort_name(fallback_thinking))
    } else if primary_is_prompt_controlled {
        "Ultrathink".into()
    } else {
        effort_name(fallback_thinking)
    };
    let both_default = name == crate::model::DEFAULT_MODEL
        && fallback_thinking.is_empty()
        && display.speed_label.is_none()
        && !display.ultracode
        && display.context_window.is_none();
    (effort, both_default, display)
}

pub(super) fn model_effort_trigger(
    s: &State,
    model: &ComposerModelDisplay,
    p: Palette,
    narrow: bool,
    locked: bool,
) -> El {
    model_trigger(s, model, p, narrow, locked, true)
}

/// Ported from KSG Code's `ComposerModelEffort` reasoning trigger: a gauge and the effort,
/// beside the model in the context strip. It opens the same model and effort picker.
/// `compact` drops the label for a narrow strip, as KSG's controls bar does.
pub(super) fn effort_trigger(
    model: &ComposerModelDisplay,
    p: Palette,
    locked: bool,
    compact: bool,
) -> Option<El> {
    let effort = &model.effort;
    if !model.show_effort() {
        return None;
    }
    let mut trigger = row()
        .key("composer-effort")
        .role("button")
        .cursor(rune::Cursor::Pointer)
        .label(format!("Reasoning {effort}. Change reasoning"))
        .help(format!("Reasoning: {effort}"))
        .preserve_focus_on_press(true)
        .center_y()
        .gap(6.0)
        .h(24.0)
        .px(7.0)
        .radius(6.0)
        .shrink()
        .min_w(0.0)
        .opacity(if locked { 0.65 } else { 1.0 })
        .disabled(locked)
        .hover(p.selected)
        .press(p.selected)
        .child(icon("gauge", p.muted).size(14.0, 14.0));
    if !compact {
        trigger = trigger.child(label(effort, SMALL, p.muted).truncate().shrink().min_w(0.0));
    }
    if !locked {
        trigger = trigger.on_press(|s: &mut State| {
            s.begin_composer_menu();
            s.open_model_picker(SettingsPicker::Thinking);
        });
    }
    Some(trigger)
}

/// The model trigger. `with_effort` keeps the effort inside it, as the resting composer does.
pub(super) fn model_trigger(
    s: &State,
    model: &ComposerModelDisplay,
    p: Palette,
    narrow: bool,
    locked: bool,
    with_effort: bool,
) -> El {
    let t = s.current();
    let (name, _) = split_model_label(&t.model);
    let effort = &model.effort;
    let display = &model.traits;
    let multi_selections = &model.selections;
    if multi_selections.len() > 1 {
        let short_labels = model
            .multi_labels
            .iter()
            .map(|(short, _)| short.as_str())
            .collect::<Vec<_>>();
        let multiple_label = if short_labels.len() > 2 {
            format!(
                "{}, {} more",
                short_labels[..2].join(", "),
                short_labels.len() - 2
            )
        } else {
            short_labels.join(", ")
        };
        let all_model_names = model
            .multi_labels
            .iter()
            .map(|(_, full)| full.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let mut icons = row().center_y().gap(2.0).shrink();
        for (sel, (_, full)) in multi_selections.iter().zip(&model.multi_labels).take(3) {
            icons = icons.child(provider_icon(s, p, model_vendor(&sel.model, full), 14.0));
        }
        let mut trigger = row()
            .key("model-effort")
            .role("button")
            .cursor(rune::Cursor::Pointer)
            .label(&all_model_names)
            .help(&all_model_names)
            .preserve_focus_on_press(true)
            .center_y()
            .gap(7.0)
            .h(28.0)
            .px(8.0)
            .radius(6.0)
            .shrink()
            .min_w(0.0)
            .max_w(if narrow { 200.0 } else { 340.0 })
            .opacity(if locked { 0.65 } else { 1.0 })
            .disabled(locked)
            .hover(p.selected)
            .press(p.selected);
        if !locked {
            trigger = trigger.on_press(|s: &mut State| {
                s.begin_composer_menu();
                s.open_model_picker(SettingsPicker::Model);
            });
        }
        trigger = trigger.child(icons);
        if !narrow {
            trigger = trigger.child(
                label(&multiple_label, TextStyle::new(13.0, 600.0), p.ink)
                    .truncate()
                    .shrink()
                    .min_w(0.0),
            );
        }
        return trigger.child(outlined(
            meta_text(format!("{}", multi_selections.len()), p.muted)
                .key("model-multi-count")
                .px(4.0)
                .py(1.0)
                .radius(6.0),
            p,
        ));
    }

    let mut trigger = row()
        .key("model-effort")
        .role("button")
        .cursor(rune::Cursor::Pointer)
        .label(format!(
            "Model {name}, reasoning {effort}. Change model or reasoning"
        ))
        .help(t.model.clone())
        .preserve_focus_on_press(true)
        .center_y()
        .gap(7.0)
        .h(28.0)
        .px(8.0)
        .radius(6.0)
        .shrink()
        .min_w(0.0)
        .max_w(if narrow { 200.0 } else { 340.0 })
        .opacity(if locked { 0.65 } else { 1.0 })
        .disabled(locked)
        .hover(p.selected)
        .press(p.selected);
    if !locked {
        trigger = trigger.on_press(|s: &mut State| {
            s.begin_composer_menu();
            s.open_model_picker(SettingsPicker::Model);
        });
    }
    let display_name = &model.trigger_label;
    let trigger = trigger.child(provider_icon(
        s,
        p,
        model_vendor(&t.model_id, &t.model),
        14.0,
    ));
    // A narrow context strip shows the model as its provider icon; the name is in the help.
    if !with_effort && narrow {
        return trigger;
    }
    let mut trigger = trigger.child(
        if with_effort {
            label(display_name, TextStyle::new(13.0, 600.0), p.ink)
        } else {
            // In the context strip the model reads like KSG's other strip controls.
            label(display_name, SMALL, p.muted)
        }
        .truncate()
        .shrink()
        .min_w(0.0),
    );
    if with_effort && !model.both_default {
        trigger = trigger.child(meta_text(effort, p.muted).truncate().shrink().min_w(0.0));
    }
    if display.speed_label.is_some() {
        trigger = trigger.child(icon("bolt", p.brass));
    }
    if display.ultracode {
        trigger = trigger.child(icon("workflow", p.sea));
    }
    if let Some(cw) = &display.context_window {
        trigger = trigger.child(meta_text(cw, p.muted).truncate().shrink().min_w(0.0));
    }
    if model.unavailable {
        trigger = trigger.child(outlined(
            meta_text("Unavailable", p.muted)
                .px(4.0)
                .py(1.0)
                .radius(6.0),
            p,
        ));
    }
    trigger
}

pub(super) const PICKER_WIDTH: f32 = 320.0;
/// KSG Code's model list column, `max-w-90`.
pub(super) const MODEL_LIST_W: f32 = 360.0;
/// KSG Code's docked options column, `w-48`.
pub(super) const MODEL_OPTIONS_W: f32 = 192.0;
/// KSG Code's docked picker height, `h-86.5`. A short window shrinks it.
pub(super) const MODEL_PICKER_H: f32 = 346.0;
/// The reasoning menu's width: KSG's `w-60` menu, widened so five levels keep room on the slider.
pub(super) const REASONING_W: f32 = 272.0;

/// The model picker or the reasoning menu, whichever is open. KSG Code keeps them apart:
/// `ProviderModelPicker` for the model and `ComposerReasoningPicker` for its levels.
pub(super) fn settings_picker(s: &State, model: &ComposerModelDisplay, p: Palette) -> El {
    if s.settings_picker == Some(SettingsPicker::Thinking) {
        reasoning_picker(s, model, p)
    } else {
        model_picker(s, model, p)
    }
}

/// A popover above the composer, right edges aligned, sized to the window (Floating UI's
/// `size` and `shift`), modal, and handing focus back to the composer when it closes. It
/// anchors to the composer rather than a trigger: the composer is always on screen, and its
/// edge stays put while a trigger's label changes width under the open menu.
pub(super) fn composer_popover(card: El) -> El {
    card.anchored_fit("composer", rune::Side::Top, rune::Align::End, 8.0)
        .focus_return("composer-input")
        .modal(true, |s: &mut State| {
            s.settings_picker = None;
            s.open_option_menu = None;
            s.thinking_drag = None;
            s.restore_composer_focus(composer_ui::FocusReason::MenuClosed);
        })
}

/// KSG's `handleInstanceModelChange`: with an options column docked the popup stays open for
/// them; otherwise picking a model closes it.
pub(super) fn finish_model_pick(s: &mut State) {
    let descriptors = s.active_model_descriptors();
    if crate::descriptors::split_column_and_footer_controls(&descriptors)
        .others
        .is_empty()
    {
        s.settings_picker = None;
        s.open_option_menu = None;
        s.restore_composer_focus(composer_ui::FocusReason::MenuClosed);
    } else {
        s.settings_picker = Some(SettingsPicker::Model);
    }
}

/// Ported from KSG Code's `ProviderModelPicker` (docked): the provider rail, search and model
/// list at one fixed height, the `ComposerEffortColumn` (`optionsOnly`) beside it when the
/// model has options besides reasoning, and the Permissions footer below (as KSG's
/// `composerControlsOwnership` keeps Permissions reachable when the default chip is quiet).
/// Below 640 px the column stacks under the list, as KSG's `max-sm` classes do.
pub(super) fn model_picker(s: &State, model: &ComposerModelDisplay, p: Palette) -> El {
    const PERMISSIONS_FOOTER_H: f32 = 37.0;
    let t = s.current();
    let can_choose = !t.status.busy() && !t.configuring;
    let stacked = s.viewport_width <= 640.0;
    let controls = crate::descriptors::split_column_and_footer_controls(&model.descriptors);
    let entries = s.provider_instance_entries();
    let options = s.picker_models_for_entries(&entries);
    let has_options = !controls.others.is_empty();
    let room = (s.viewport_width - 16.0).max(240.0);
    let width = if stacked {
        room
    } else if has_options {
        MODEL_LIST_W + 1.0 + MODEL_OPTIONS_W
    } else {
        MODEL_LIST_W
    }
    .min(room);

    // Docked beside options, KSG holds one fixed height; alone, the list is as tall as its
    // tallest tab, so switching tabs or searching never resizes it.
    // The card is anchored above the composer (`anchored_fit`, 8 pt gutter): the
    // room it leaves bounds the column, so a clamped card never overflows its
    // own rows. One frame behind, as `anchored_fit` measures it.
    let above = rune::memo::layout("composer")
        .map(|rect| rect[1] - 16.0 - PERMISSIONS_FOOTER_H)
        .unwrap_or(f32::MAX);
    let list_h = (if stacked && has_options {
        224.0
    } else if has_options {
        MODEL_PICKER_H
    } else {
        tallest_model_tab_h(s, &options, entries.len()).min(if stacked {
            320.0
        } else {
            MODEL_PICKER_H
        })
    })
    .min(above.max(120.0));
    let list_col = col()
        .key("model-picker-list-col")
        .w(if stacked {
            width
        } else {
            MODEL_LIST_W.min(width)
        })
        .h(list_h)
        .shrink()
        .min_w(0.0)
        .min_h(0.0);
    let snapshot = picker_snapshot_from(s, entries, &options);
    let list_col = model_list(
        s,
        p,
        can_choose,
        list_col,
        &snapshot,
        &model.selections,
        !(stacked && has_options),
    );
    let mut body = if stacked { col() } else { row() }
        .key("model-picker-body")
        .w_full()
        .min_w(0.0)
        .min_h(0.0)
        .shrink()
        .child(list_col);
    if has_options {
        body = body
            .child(if stacked {
                rule(p)
            } else {
                el().w(1.0).h_full().fill(p.line).hairline()
            })
            .child(options_column(
                s,
                p,
                can_choose,
                stacked,
                &model.label,
                &controls,
            ));
    }
    let display_access = s.effective_access_mode(t);
    let access_disabled = t.status.busy() || t.configuring || t.archived();
    let access_emphasized = display_access.id() != s.thread_defaults.access;
    let mut permissions_row = row()
        .key("model-picker-permissions")
        .role("button")
        .label(format!("Permissions: {}", display_access.label()))
        .w_full()
        .h(36.0)
        .px(12.0)
        .gap(8.0)
        .center_y()
        .disabled(access_disabled)
        .opacity(if access_disabled { 0.65 } else { 1.0 })
        .child(
            icon(
                pi_ui::access_glyph(display_access),
                if access_emphasized { p.ink } else { p.muted },
            )
            .size(14.0, 14.0),
        )
        .child(label("Permissions", SMALL, p.ink))
        .child(spacer())
        .child(label(display_access.label(), META, p.muted))
        .child(icon("chevron-right", p.faint).size(12.0, 12.0));
    if !access_disabled {
        permissions_row = permissions_row
            .cursor(rune::Cursor::Pointer)
            .focusable(true)
            .hover(p.selected)
            .press(p.selected)
            .on_press(|s: &mut State| {
                s.settings_picker = None;
                s.open_access_picker();
            });
    }
    let mut card = col()
        .key("pi-settings-picker")
        .role("dialog")
        .label("Choose a model")
        .w(width)
        .min_h(0.0)
        .radius(12.0)
        .fill(p.card)
        .border(1.0, p.frame)
        .box_shadow(soft_shadow(p))
        .on_press(|_: &mut State| {})
        .child(body)
        .child(rule(p))
        .child(permissions_row);
    if let Some(error) = &t.error {
        card = card.child(rule(p)).child(
            label(error, SMALL, p.danger)
                .wrap_anywhere()
                .px(12.0)
                .py(8.0),
        );
    }
    if let Some(open_id) = &s.open_option_menu
        && let Some(desc) = controls.others.iter().find(|d| d.id() == open_id)
    {
        card = card.child(option_choice_menu(desc, p, can_choose, stacked));
    }
    composer_popover(card)
}

/// The model list's height for its tallest provider tab or favorites, from KSG's
/// `tallestTabUnits`: a one-line row is 32 pt, a two-line row 48 pt, 2 pt apart, plus the
/// search row and the list's padding. It never drops below the provider rail.
pub(super) type PickerModels =
    std::collections::BTreeMap<String, Vec<crate::model_picker::PickerModelOption>>;

pub(super) fn tallest_model_tab_h(s: &State, options: &PickerModels, entry_count: usize) -> f32 {
    use crate::model_picker::provider_model_key;
    const CHROME: f32 = 51.0 + 12.0;
    let favorites: std::collections::HashSet<String> = s
        .effective_model_favorites()
        .iter()
        .map(|f| provider_model_key(&f.provider, &f.model))
        .collect();
    let row = |two_line: bool| if two_line { 48.0 } else { 32.0 };
    let mut tallest: f32 = 0.0;
    let mut favorites_h: f32 = 0.0;
    for (instance, models) in options {
        let mut tab_h: f32 = 0.0;
        let mut legacy = false;
        for model in models {
            let favorite = favorites.contains(&provider_model_key(instance, &model.slug));
            if favorite {
                favorites_h += row(true) + 2.0;
            }
            if model.is_legacy && !favorite {
                legacy = true;
            } else {
                tab_h += row(model.sub_provider.is_some()) + 2.0;
            }
        }
        if legacy {
            tab_h += row(false) + 2.0;
        }
        tallest = tallest.max(tab_h);
    }
    let rail = (entry_count + 1) as f32 * 40.0 + 8.0;
    (CHROME + tallest.max(favorites_h)).max(rail).max(160.0)
}

/// An option row's choices, beside the row as KSG's `OptionMenuRow` opens them.
pub(super) fn option_choice_menu(
    desc: &crate::descriptors::ProviderOptionDescriptor,
    p: Palette,
    can_choose: bool,
    stacked: bool,
) -> El {
    let crate::descriptors::ProviderOptionDescriptor::Select {
        id,
        options,
        current_value,
        ..
    } = desc
    else {
        return el();
    };
    let choices = crate::descriptors::selectable_provider_option_choices(desc);
    let current_id = current_value
        .as_deref()
        .or_else(|| options.iter().find(|o| o.is_default).map(|o| o.id.as_str()))
        .unwrap_or("");
    let mut choice_col = col()
        .key(format!("option-popup-{id}"))
        .w(220.0)
        .padding(4.0)
        .gap(2.0)
        .radius(12.0)
        .fill(p.card)
        .border(1.0, p.frame)
        .box_shadow(soft_shadow(p));
    for choice in choices {
        let selected = choice.id == current_id;
        let oid = id.clone();
        let cid = choice.id.clone();
        let mut crow = row()
            .key(format!("option-choice-{id}-{}", choice.id))
            .role("menuitemradio")
            .checked(selected)
            .focusable(can_choose)
            .px(8.0)
            .py(5.0)
            .gap(6.0)
            .center_y()
            .radius(6.0)
            .fill(if selected {
                p.selected
            } else {
                Color::TRANSPARENT
            })
            .hover(p.selected)
            .cursor(rune::Cursor::Pointer)
            .on_press(move |s: &mut State| {
                s.set_composer_option_sticky(
                    oid.clone(),
                    crate::model_selection::OptionValue::Str(cid.clone()),
                );
                s.open_option_menu = None;
            })
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .child(label(&choice.label, SMALL, p.ink).truncate())
                    .child(if let Some(d) = &choice.description {
                        label(d, META, p.muted).wrap()
                    } else {
                        el()
                    }),
            );
        if choice.is_default {
            crow = crow.child(meta_text("Default", p.muted));
        }
        if selected {
            crow = crow.child(icon("check", p.ink).size(14.0, 14.0));
        }
        choice_col = choice_col.child(crow);
    }
    choice_col.anchored_fit(
        format!("option-menu-{id}"),
        if stacked {
            rune::Side::Bottom
        } else {
            rune::Side::Right
        },
        rune::Align::Start,
        6.0,
    )
}

/// Ported from KSG Code's `ComposerEffortColumn` with `optionsOnly`: the model's options other
/// than reasoning and speed, one row each, under a header that names the model.
pub(super) fn options_column(
    s: &State,
    p: Palette,
    can_choose: bool,
    stacked: bool,
    model_label: &str,
    controls: &crate::descriptors::ColumnControls,
) -> El {
    // KSG's `bg-muted/40`, a shade off the card.
    let tint = Color::BLACK.alpha(if s.dark { 0.12 } else { 0.03 });
    let mut column = col()
        .key("model-picker-options")
        .h(if stacked { 256.0 } else { MODEL_PICKER_H })
        .shrink()
        .min_h(0.0)
        .min_w(0.0)
        .fill(tint);
    column = if stacked {
        column.w_full().corners([0.0, 0.0, 11.0, 11.0])
    } else {
        column.w(MODEL_OPTIONS_W).corners([0.0, 11.0, 11.0, 0.0])
    };
    // The search row's height, so one rule runs across the top of the popup.
    column = column
        .child(
            row()
                .center_y()
                .gap(8.0)
                .h(50.0)
                .px(12.0)
                .child(label("Options", META, p.muted))
                .child(spacer())
                .child(
                    label(model_label, META, p.muted)
                        .key("effort-model-label")
                        .truncate()
                        .shrink()
                        .min_w(0.0),
                ),
        )
        .child(rule(p));
    column.child(
        col()
            .w_full()
            .gap(2.0)
            .px(8.0)
            .pt(10.0)
            .pb(6.0)
            .h(0.0)
            .grow(1.0)
            .min_h(0.0)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .children(option_rows(s, p, can_choose, controls)),
    )
}

/// KSG's `OptionSwitchRow` and `OptionMenuRow`: an option's name over its current value.
pub(super) fn option_rows(
    s: &State,
    p: Palette,
    can_choose: bool,
    controls: &crate::descriptors::ColumnControls,
) -> Vec<El> {
    let mut rows = Vec::new();
    for desc in &controls.others {
        match desc {
            crate::descriptors::ProviderOptionDescriptor::Boolean {
                id,
                label: opt_label,
                current_value,
                ..
            } => {
                let on = *current_value == Some(true);
                let oid = id.clone();
                rows.push(
                    row()
                        .key(format!("option-switch-{id}"))
                        .role("switch")
                        .checked(on)
                        .label(format!("{opt_label}: {}", if on { "On" } else { "Off" }))
                        .focusable(can_choose)
                        .disabled(!can_choose)
                        .px(8.0)
                        .py(4.0)
                        .gap(8.0)
                        .center_y()
                        .radius(6.0)
                        .hover(p.selected)
                        .focus_fill(p.selected)
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |s: &mut State| {
                            s.set_composer_option_sticky(
                                oid.clone(),
                                crate::model_selection::OptionValue::Bool(!on),
                            );
                        })
                        .child(option_row_text(opt_label, if on { "On" } else { "Off" }, p))
                        .child(switch_track(on, p)),
                );
            }
            crate::descriptors::ProviderOptionDescriptor::Select {
                id,
                label: opt_label,
                options,
                current_value,
                ..
            } => {
                let choices = crate::descriptors::selectable_provider_option_choices(desc);
                let current_id = current_value
                    .as_deref()
                    .or_else(|| options.iter().find(|o| o.is_default).map(|o| o.id.as_str()))
                    .unwrap_or("");
                let current_label = choices
                    .iter()
                    .find(|c| c.id == current_id)
                    .map(|c| c.label.as_str())
                    .unwrap_or(current_id);
                let is_open = s.open_option_menu.as_deref() == Some(id.as_str());
                let oid = id.clone();
                rows.push(
                    row()
                        .key(format!("option-menu-{id}"))
                        .role("button")
                        .label(format!("{opt_label}: {current_label}"))
                        .focusable(can_choose)
                        .disabled(!can_choose)
                        .px(8.0)
                        .py(4.0)
                        .gap(6.0)
                        .center_y()
                        .radius(6.0)
                        .fill(if is_open {
                            p.selected
                        } else {
                            Color::TRANSPARENT
                        })
                        .hover(p.selected)
                        .focus_fill(p.selected)
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |s: &mut State| {
                            if s.open_option_menu.as_deref() == Some(oid.as_str()) {
                                s.open_option_menu = None;
                            } else {
                                s.open_option_menu = Some(oid.clone());
                            }
                        })
                        .child(option_row_text(opt_label, current_label, p))
                        .child(icon("open", p.muted).size(14.0, 14.0)),
                );
            }
        }
    }
    rows
}

/// KSG's `OptionRowText`: the option's name in meta type over its value.
pub(super) fn option_row_text(name: &str, value: &str, p: Palette) -> El {
    col()
        .grow(1.0)
        .min_w(0.0)
        .child(label(name, META, p.muted).truncate())
        .child(label(value, TextStyle::new(13.0, 500.0), p.ink).truncate())
}

/// KSG's small `Switch`: a 28 x 16 track with a 12 pt knob.
pub(super) fn switch_track(on: bool, p: Palette) -> El {
    let mut track = row()
        .w(28.0)
        .h(16.0)
        .radius(8.0)
        .padding(2.0)
        .center_y()
        .shrink()
        .fill(if on { p.coral } else { p.line });
    if on {
        track = track.justify_end();
    }
    track.child(el().size(12.0, 12.0).radius(6.0).fill(p.card))
}

/// Ported from KSG Code's `ComposerReasoningPicker`: its own menu above the effort chip, with
/// the reasoning levels under a header that names the model and the speed controls below
/// them. The levels keep Shina's Codex slider, laid out for this narrower menu.
pub(super) fn reasoning_picker(s: &State, model: &ComposerModelDisplay, p: Palette) -> El {
    let t = s.current();
    let can_choose = !t.status.busy() && !t.configuring;
    let controls = crate::descriptors::split_column_and_footer_controls(&model.descriptors);
    let title = controls
        .effort
        .as_ref()
        .map(|d| d.label().to_owned())
        .unwrap_or_else(|| "Reasoning".into());
    let model_label = &model.label;
    let card = col()
        .key("reasoning-picker")
        .role("dialog")
        .label("Reasoning")
        .w(REASONING_W.min((s.viewport_width - 16.0).max(200.0)))
        .min_h(0.0)
        .radius(12.0)
        .fill(p.card)
        .border(1.0, p.frame)
        .box_shadow(soft_shadow(p))
        .on_press(|_: &mut State| {})
        .child(
            row()
                .key("reasoning-header")
                .center_y()
                .gap(8.0)
                .px(12.0)
                .pt(10.0)
                .pb(2.0)
                .child(label(&title, META, p.muted))
                .child(spacer())
                .child(
                    label(model_label, META, p.muted)
                        .key("effort-model-label")
                        .truncate()
                        .shrink()
                        .min_w(0.0),
                ),
        );
    // Below the header everything scrolls, so a window too short for the menu clips nothing.
    let mut body = col()
        .key("reasoning-body")
        .w_full()
        .shrink()
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(effort_body(s, p, can_choose, &controls));
    if let Some(speed) = speed_section(s, p, can_choose, &controls) {
        body = body.child(rule(p)).child(speed);
    }
    if let Some(error) = &t.error {
        body = body.child(rule(p)).child(
            label(error, SMALL, p.danger)
                .wrap_anywhere()
                .px(12.0)
                .py(8.0),
        );
    }
    composer_popover(card.child(body))
}

/// The reasoning levels: the slider for a select, KSG's On and Off rows for a boolean, or a
/// note when the model has none.
pub(super) fn effort_body(
    s: &State,
    p: Palette,
    can_choose: bool,
    controls: &crate::descriptors::ColumnControls,
) -> El {
    let t = s.current();
    // The drag handler re-checks the lock live, since "ultrathink" can land in the draft
    // mid-gesture. `locks_when_body` is the descriptor half of the predicate.
    let locks_when_body = controls.effort.as_ref().is_some_and(|d| match d {
        crate::descriptors::ProviderOptionDescriptor::Select {
            prompt_injected_values,
            ..
        } => !prompt_injected_values.is_empty(),
        _ => false,
    });
    let ultrathink_in_body = locks_when_body
        && crate::descriptors::read_prompt_ultrathink(&t.draft.text)
            == crate::descriptors::PromptUltrathink::Body;

    let mut body = col().key("model-picker-effort").w_full();
    match &controls.effort {
        Some(
            desc @ crate::descriptors::ProviderOptionDescriptor::Select {
                id, current_value, ..
            },
        ) => {
            let choices = crate::descriptors::selectable_provider_option_choices(desc);
            let levels: Vec<String> = if choices.is_empty() {
                t.thinking_levels.clone()
            } else {
                choices.into_iter().map(|c| c.id).collect()
            };
            // One consistent value renders the level: the pinned pick when the user just
            // chose, else the descriptor's current value, else what the provider runs.
            let current = t
                .selected_thinking
                .clone()
                .filter(|v| levels.iter().any(|l| l == v))
                .or_else(|| {
                    current_value
                        .clone()
                        .filter(|v| levels.iter().any(|l| l == v))
                })
                .unwrap_or_else(|| t.thinking.clone());
            if ultrathink_in_body {
                body = body.child(
                    label(crate::descriptors::ULTRATHINK_IN_BODY_NOTE, META, p.muted)
                        .key("ultrathink-lock-note")
                        .wrap()
                        .px(12.0)
                        .pt(6.0),
                );
            }
            body.child(effort_slider_with_levels(
                &levels,
                &current,
                s.thinking_drag,
                p,
                t.can_choose_thinking() && !ultrathink_in_body,
                id.clone(),
                format!("{}:{}:{}", t.id, t.harness, t.model_id),
                locks_when_body,
            ))
        }
        Some(crate::descriptors::ProviderOptionDescriptor::Boolean {
            id, current_value, ..
        }) => {
            let on = *current_value == Some(true);
            let mut group = col().role("radiogroup").gap(2.0).padding(6.0).w_full();
            for (label_text, val) in [("On", true), ("Off", false)] {
                let checked = on == val;
                let oid = id.clone();
                group = group.child(
                    row()
                        .key(format!("effort-bool-{id}-{val}"))
                        .role("radio")
                        .checked(checked)
                        .focusable(can_choose)
                        .disabled(!can_choose)
                        .h(30.0)
                        .px(8.0)
                        .center_y()
                        .radius(6.0)
                        .hover(p.selected)
                        .focus_fill(p.selected)
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |s: &mut State| {
                            s.set_composer_option_sticky(
                                oid.clone(),
                                crate::model_selection::OptionValue::Bool(val),
                            );
                        })
                        .child(label(label_text, SMALL, p.ink).grow(1.0))
                        .child(if checked {
                            icon("check", p.ink).size(14.0, 14.0)
                        } else {
                            el()
                        }),
                );
            }
            body.child(group)
        }
        None if !t.thinking_levels.is_empty() => {
            let fallback_id = match t.harness.as_str() {
                "pi" => "thinking",
                "codex" => "reasoningEffort",
                _ => "effort",
            };
            body.child(effort_slider_with_levels(
                &t.thinking_levels,
                t.selected_thinking.as_deref().unwrap_or(&t.thinking),
                s.thinking_drag,
                p,
                t.can_choose_thinking(),
                fallback_id.into(),
                format!("{}:{}:{}", t.id, t.harness, t.model_id),
                false,
            ))
        }
        None => body.child(
            label("No effort levels", SMALL, p.muted)
                .key("no-effort-levels")
                .px(12.0)
                .py(10.0),
        ),
    }
}

/// Ported from KSG Code's `SpeedControl` and `UltracodeControl`, which its reasoning menu keeps
/// below the levels: service tiers, Claude's Fast mode, or Codex's legacy tiers, then Ultracode.
pub(super) fn speed_section(
    s: &State,
    p: Palette,
    can_choose: bool,
    controls: &crate::descriptors::ColumnControls,
) -> Option<El> {
    let t = s.current();
    let segment = |key: String, text: &str, on: bool| {
        row()
            .key(key)
            .role("radio")
            .label(text)
            .checked(on)
            .focusable(can_choose)
            .disabled(!can_choose)
            .h(24.0)
            .w(0.0)
            .grow(1.0)
            .min_w(0.0)
            .px(8.0)
            .gap(4.0)
            .center()
            .radius(6.0)
            .fill(if on { p.card } else { Color::TRANSPARENT })
            .hover(if on { p.card } else { p.selected })
            .focus_fill(p.selected)
            .cursor(rune::Cursor::Pointer)
    };
    let group = |key: &str, name: &str| {
        row()
            .key(key)
            .role("radiogroup")
            .label(name)
            .w_full()
            .gap(2.0)
            .padding(2.0)
            .radius(8.0)
            .fill(p.selected)
    };
    let mut section = col()
        .key("picker-footer")
        .w_full()
        .gap(8.0)
        .px(12.0)
        .py(10.0);
    let mut any = false;
    if let Some(crate::descriptors::ProviderOptionDescriptor::Select {
        options,
        current_value,
        ..
    }) = &controls.speed_select
    {
        let selected = t.harness_state["serviceTier"]
            .as_str()
            .or_else(|| current_value.as_deref())
            .unwrap_or("default");
        let mut tiers = group("service-tier-picker", "Service Tier");
        for (index, opt) in options.iter().enumerate() {
            let on = selected == opt.id;
            let oid = opt.id.clone();
            let mut seg = segment(format!("service-tier-{index}"), &opt.label, on).on_press(
                move |s: &mut State| {
                    s.set_composer_service_tier_sticky(oid.clone());
                },
            );
            if let Some(desc) = &opt.description {
                seg = seg.help(desc);
            }
            if opt.label == "Fast" || opt.label == "Ultrafast" {
                seg = seg.child(icon("bolt", if on { p.brass } else { p.muted }).size(14.0, 14.0));
            }
            tiers = tiers.child(
                seg.child(label(&opt.label, SMALL, if on { p.ink } else { p.muted }).truncate()),
            );
        }
        section = section.child(label("Speed", META, p.muted)).child(tiers);
        any = true;
    } else if let Some(tiers) = service_tier_picker(t, p, can_choose) {
        section = section.child(tiers);
        any = true;
    } else if let Some(crate::descriptors::ProviderOptionDescriptor::Boolean {
        current_value,
        ..
    }) = &controls.fast_toggle
    {
        let is_fast = *current_value == Some(true);
        let tiers = group("fast-mode-picker", "Fast Mode")
            .child(
                segment("fast-mode-off".into(), "Standard", !is_fast)
                    .on_press(|s: &mut State| {
                        s.set_composer_option_sticky(
                            "fastMode".into(),
                            crate::model_selection::OptionValue::Bool(false),
                        );
                    })
                    .child(label(
                        "Standard",
                        SMALL,
                        if !is_fast { p.ink } else { p.muted },
                    )),
            )
            .child(
                segment("fast-mode-on".into(), "Fast", is_fast)
                    .on_press(|s: &mut State| {
                        s.set_composer_option_sticky(
                            "fastMode".into(),
                            crate::model_selection::OptionValue::Bool(true),
                        );
                    })
                    .child(icon("bolt", if is_fast { p.brass } else { p.muted }).size(14.0, 14.0))
                    .child(label("Fast", SMALL, if is_fast { p.ink } else { p.muted })),
            );
        section = section.child(label("Speed", META, p.muted)).child(tiers);
        any = true;
    }

    if let Some(crate::descriptors::ProviderOptionDescriptor::Boolean {
        id,
        label: ultra_label,
        current_value,
        ..
    }) = &controls.ultracode_toggle
    {
        let on = *current_value == Some(true);
        let oid = id.clone();
        section = section.child(
            row()
                .key("ultracode-toggle")
                .role("switch")
                .label(ultra_label)
                .checked(on)
                .focusable(can_choose)
                .disabled(!can_choose)
                .h(28.0)
                .mx(-4.0)
                .px(4.0)
                .gap(8.0)
                .center_y()
                .radius(6.0)
                .hover(p.selected)
                .focus_fill(p.selected)
                .cursor(rune::Cursor::Pointer)
                .on_press(move |s: &mut State| {
                    s.set_composer_option_sticky(
                        oid.clone(),
                        crate::model_selection::OptionValue::Bool(!on),
                    );
                })
                .child(icon("workflow", if on { p.sea } else { p.muted }).size(14.0, 14.0))
                .child(
                    label(ultra_label, SMALL, p.ink)
                        .truncate()
                        .grow(1.0)
                        .min_w(0.0),
                )
                .child(switch_track(on, p)),
        );
        any = true;
    }
    any.then_some(section)
}

/// Codex's legacy service tiers, read from the model, as KSG's `SpeedControl` segments.
pub(super) fn service_tier_picker(t: &Thread, p: Palette, can_choose: bool) -> Option<El> {
    let model = t
        .models
        .iter()
        .find(|model| model.id == t.model_id && model.provider == "codex")?;
    if model.service_tiers.is_empty() {
        return None;
    }
    let mut choices = row()
        .key("service-tier-options")
        .role("radiogroup")
        .label("Service tier")
        .w_full()
        .gap(2.0)
        .padding(2.0)
        .radius(8.0)
        .fill(p.selected);
    let selected = t.harness_state["serviceTier"]
        .as_str()
        .or_else(|| t.harness_state["effectiveServiceTier"].as_str())
        .or_else(|| {
            model
                .service_tiers
                .iter()
                .find(|tier| tier["isDefault"] == true)
                .and_then(|tier| tier["id"].as_str())
        })
        .unwrap_or("default");
    for (index, tier) in model.service_tiers.iter().enumerate() {
        let Some(id) = tier["id"].as_str() else {
            continue;
        };
        let name = tier["name"].as_str().unwrap_or(id);
        let id = id.to_owned();
        let on = selected == id;
        choices = choices.child(
            row()
                .key(format!("service-tier-{index}"))
                .role("radio")
                .label(name)
                .checked(on)
                .focusable(can_choose)
                .disabled(!can_choose)
                .h(24.0)
                .w(0.0)
                .grow(1.0)
                .min_w(0.0)
                .px(8.0)
                .center_y()
                .justify_center()
                .radius(6.0)
                .fill(if on { p.card } else { Color::TRANSPARENT })
                .hover(if on { p.card } else { p.selected })
                .focus_fill(p.selected)
                .cursor(rune::Cursor::Pointer)
                .child(label(name, SMALL, if on { p.ink } else { p.muted }).truncate())
                .on_press(move |s: &mut State| {
                    s.set_composer_service_tier_sticky(id.clone());
                }),
        );
    }
    Some(
        col()
            .key("service-tier-picker")
            .gap(8.0)
            .child(label("Service tier", META, p.muted))
            .child(choices),
    )
}

pub(super) struct PickerSnapshot {
    pub(super) entries: Vec<crate::model_picker::ProviderInstanceEntry>,
    pub(super) sidebar_entries: Vec<crate::model_picker::ProviderInstanceEntry>,
    pub(super) selected_tab: crate::model_picker::ModelPickerTab,
    pub(super) is_searching: bool,
    pub(super) locked_disabled_ids: std::collections::HashSet<String>,
    pub(super) selectable_unavailable_ids: std::collections::HashSet<String>,
    pub(super) favorites_set: std::collections::HashSet<String>,
    pub(super) visible_models: Vec<crate::model_picker::ModelPickerItem>,
    pub(super) legacy_section: Option<(String, bool)>,
    pub(super) setup_entries: Vec<crate::model_picker::ProviderInstanceEntry>,
    pub(super) update_required_notices: Vec<(String, String)>,
}

pub(super) fn picker_snapshot(s: &State) -> PickerSnapshot {
    let entries = s.provider_instance_entries();
    let options = s.picker_models_for_entries(&entries);
    picker_snapshot_from(s, entries, &options)
}

pub(super) fn picker_snapshot_from(
    s: &State,
    entries: Vec<crate::model_picker::ProviderInstanceEntry>,
    options_by_instance: &PickerModels,
) -> PickerSnapshot {
    use crate::model_picker::{
        ModelPickerItem, ModelPickerSearchableModel, ModelPickerTab,
        is_provider_instance_picker_ready, is_provider_instance_picker_visible,
        matches_locked_provider, provider_model_key, rank_model_picker_search,
        should_include_model_picker_option, should_offer_model_picker_setup,
        sort_provider_model_items, split_legacy_models,
    };

    let t = s.current();
    let favorites = s.effective_model_favorites();
    let favorites_set: std::collections::HashSet<String> = favorites
        .iter()
        .map(|f| provider_model_key(&f.provider, &f.model))
        .collect();
    let locked_provider = s.locked_provider_for_current();
    let is_searching = !s.model_search.text.trim().is_empty();

    let selected_tab = s
        .model_picker_tab
        .clone()
        .unwrap_or_else(|| ModelPickerTab::Instance(t.harness.clone()));

    let mut locked_disabled_ids = std::collections::HashSet::new();
    if locked_provider.is_some() {
        for entry in &entries {
            if !matches_locked_provider(entry, locked_provider.as_deref(), None) {
                locked_disabled_ids.insert(entry.instance_id.clone());
            }
        }
    }

    let mut selectable_unavailable_ids = std::collections::HashSet::new();
    if let Some(active_entry) = entries.iter().find(|e| e.instance_id == t.harness)
        && !is_provider_instance_picker_ready(active_entry)
        && options_by_instance.get(&t.harness).is_some_and(|opts| {
            opts.iter().any(|opt| {
                should_include_model_picker_option(active_entry, opt, &t.harness, &t.model_id)
            })
        })
    {
        selectable_unavailable_ids.insert(t.harness.clone());
    }
    for entry in &entries {
        let opts = options_by_instance
            .get(&entry.instance_id)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        if should_offer_model_picker_setup(entry, opts) {
            selectable_unavailable_ids.insert(entry.instance_id.clone());
        }
    }

    let sidebar_entries = {
        let visible: Vec<_> = entries
            .iter()
            .filter(|e| is_provider_instance_picker_visible(e))
            .cloned()
            .collect();
        if locked_provider.is_none() {
            visible
        } else {
            let mut available = Vec::new();
            let mut disabled = Vec::new();
            for entry in visible {
                if matches_locked_provider(&entry, locked_provider.as_deref(), None) {
                    available.push(entry);
                } else {
                    disabled.push(entry);
                }
            }
            available.extend(disabled);
            available
        }
    };

    let mut flat_models = Vec::new();
    for entry in &entries {
        let opts = options_by_instance
            .get(&entry.instance_id)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        for (catalog_index, model) in opts.iter().enumerate() {
            if !should_include_model_picker_option(entry, model, &t.harness, &t.model_id) {
                continue;
            }
            flat_models.push(ModelPickerItem {
                slug: model.slug.clone(),
                name: model.name.clone(),
                short_name: model.short_name.clone(),
                sub_provider: model.sub_provider.clone(),
                badge: model.badge.clone(),
                is_legacy: model.is_legacy,
                is_unavailable: model.is_unavailable,
                instance_id: entry.instance_id.clone(),
                driver_kind: entry.driver_kind.clone(),
                instance_display_name: entry.display_name.clone(),
                continuation_group_key: entry.continuation_group_key.clone(),
                catalog_index,
            });
        }
    }

    let instance_order: Vec<String> = entries.iter().map(|e| e.instance_id.clone()).collect();
    let entry_by_id: std::collections::HashMap<&str, &crate::model_picker::ProviderInstanceEntry> =
        entries
            .iter()
            .map(|e| (e.instance_id.as_str(), e))
            .collect();

    let filtered_models: Vec<ModelPickerItem> = if is_searching {
        let candidates: Vec<ModelPickerItem> = if locked_provider.is_some() {
            flat_models
                .into_iter()
                .filter(|m| {
                    entry_by_id.get(m.instance_id.as_str()).is_some_and(|e| {
                        matches_locked_provider(e, locked_provider.as_deref(), None)
                    })
                })
                .collect()
        } else {
            flat_models
        };
        rank_model_picker_search(&candidates, &s.model_search.text, |m| {
            ModelPickerSearchableModel {
                driver_kind: &m.driver_kind,
                provider_display_name: &m.instance_display_name,
                name: &m.name,
                short_name: m.short_name.as_deref(),
                sub_provider: m.sub_provider.as_deref(),
                is_favorite: favorites_set.contains(&provider_model_key(&m.instance_id, &m.slug)),
            }
        })
        .into_iter()
        .cloned()
        .collect()
    } else {
        let scoped: Vec<ModelPickerItem> = flat_models
            .into_iter()
            .filter(|m| {
                if locked_provider.is_some()
                    && !entry_by_id.get(m.instance_id.as_str()).is_some_and(|e| {
                        matches_locked_provider(e, locked_provider.as_deref(), None)
                    })
                {
                    return false;
                }
                match &selected_tab {
                    ModelPickerTab::Favorites => {
                        favorites_set.contains(&provider_model_key(&m.instance_id, &m.slug))
                    }
                    ModelPickerTab::Instance(id) => &m.instance_id == id,
                }
            })
            .collect();
        let is_fav_tab = matches!(selected_tab, ModelPickerTab::Favorites);
        sort_provider_model_items(
            &scoped,
            |m| &m.instance_id,
            |m| &m.slug,
            &favorites_set,
            !is_fav_tab,
            if is_fav_tab { &instance_order } else { &[] },
        )
        .into_iter()
        .cloned()
        .collect()
    };

    let is_fav_tab = matches!(selected_tab, ModelPickerTab::Favorites);
    let (current_models, legacy_models) =
        split_legacy_models(filtered_models, is_searching, is_fav_tab, &favorites_set);
    let legacy_section = match &selected_tab {
        ModelPickerTab::Instance(id) if !legacy_models.is_empty() => {
            Some((id.clone(), s.expanded_legacy_instances.contains(id)))
        }
        _ => None,
    };
    let mut visible_models = current_models;
    if let Some((_, true)) = legacy_section {
        visible_models.extend(legacy_models);
    }

    let selected_instance_id = match &selected_tab {
        ModelPickerTab::Instance(id) => Some(id.as_str()),
        ModelPickerTab::Favorites => None,
    };
    let setup_entries: Vec<_> = if !is_searching {
        entries
            .iter()
            .filter(|entry| {
                let opts = options_by_instance
                    .get(&entry.instance_id)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]);
                matches_locked_provider(entry, locked_provider.as_deref(), None)
                    && should_offer_model_picker_setup(entry, opts)
                    && selected_instance_id.is_none_or(|id| entry.instance_id == id)
                    && (selected_instance_id.is_some() || visible_models.is_empty())
            })
            .cloned()
            .collect()
    } else {
        vec![]
    };

    let mut update_required_notices = Vec::new();
    for entry in &entries {
        let shown = if is_searching {
            matches_locked_provider(entry, locked_provider.as_deref(), None)
        } else {
            selected_instance_id == Some(entry.instance_id.as_str())
        };
        if shown
            && let Some(notice) = crate::model_picker::format_provider_update_required_notice(
                &entry.driver_kind,
                &entry.display_name,
                &entry.update_required_models,
                &s.model_search.text,
            )
        {
            update_required_notices.push((entry.instance_id.clone(), notice));
        }
    }

    PickerSnapshot {
        entries,
        sidebar_entries,
        selected_tab,
        is_searching,
        locked_disabled_ids,
        selectable_unavailable_ids,
        favorites_set,
        visible_models,
        legacy_section,
        setup_entries,
        update_required_notices,
    }
}

/// Ported from KSG Code's `ModelPickerContent.tsx`, `ModelPickerSidebar.tsx`, and `ModelListRow.tsx`.
/// `round_bottom` rounds the rail's lower corner, when the list ends at the popup's bottom.
pub(super) fn model_list(
    s: &State,
    p: Palette,
    can_choose: bool,
    card: El,
    snap: &PickerSnapshot,
    selections: &[crate::model_selection::ModelSelection],
    round_bottom: bool,
) -> El {
    use crate::model_picker::{
        ModelPickerTab, describe_unavailable_instance, get_display_model_name,
        get_provider_status_message, is_provider_instance_picker_ready, provider_model_key,
    };

    let t = s.current();
    let show_sidebar = !snap.is_searching && !snap.sidebar_entries.is_empty();

    let search_field = text_input(&s.model_search, TextStyle::new(13.0, 400.0))
        .key("model-search")
        .label("Search models")
        .placeholder("Search models...", p.muted)
        .color(p.ink)
        .caret(p.ink, p.coral_bg)
        .grow(1.0)
        .min_w(0.0)
        .on_edit(|s: &mut State| &mut s.model_search)
        // Down from the search field goes into the list, as in KSG Code's
        // combobox; the list's own group takes the arrows from there.
        .on_input_key(|s: &mut State, event, tx| {
            if (event.mods.cmd || event.mods.ctrl || event.mods.alt) && crate::key(s, event, tx) {
                return true;
            }
            if matches!(event.key, rune::Key::Down)
                && !event.mods.cmd
                && !event.mods.ctrl
                && !event.mods.alt
                && !event.mods.shift
            {
                // The field's own focus flag would win over `tx.focus`.
                s.model_search.focused = false;
                tx.focus("model-list", true);
                return true;
            }
            false
        })
        .autofocus();

    let header = row().center_y().gap(8.0).h(50.0).px(10.0).shrink().child(
        row()
            .center_y()
            .gap(6.0)
            .h(30.0)
            .px(10.0)
            .radius(6.0)
            .fill(p.selected)
            .field_focus(&s.model_search, p)
            .grow(1.0)
            .min_w(0.0)
            .child(icon("search", p.muted))
            .child(search_field),
    );

    // KSG's `--model-picker-rail`: a darker strip of square provider buttons, the selected one
    // filled.
    let mut sidebar = col()
        .key("harness-group")
        .role("toolbar")
        .label("Providers")
        .w(44.0)
        .h_full()
        .min_h(0.0)
        .padding(4.0)
        .gap(4.0)
        // Inside the popup's 1 pt border, so its 12 pt corners less one.
        .corners([11.0, 0.0, 0.0, if round_bottom { 11.0 } else { 0.0 }])
        .fill(Color::BLACK.alpha(if s.dark { 0.2 } else { 0.1 }))
        .scroll(ScrollSpec {
            stick_bottom: false,
        });
    let fav_selected = snap.selected_tab == ModelPickerTab::Favorites;
    sidebar = sidebar.child(
        row()
            .key("choose-harness-favorites")
            .role("button")
            .label("Favorites")
            .help("Favorites")
            .checked(fav_selected)
            .cursor(rune::Cursor::Pointer)
            .size(36.0, 36.0)
            .center()
            .radius(8.0)
            .fill(if fav_selected {
                p.selected
            } else {
                Color::TRANSPARENT
            })
            .hover(p.selected)
            .on_press(|s: &mut State| {
                s.model_picker_tab = Some(ModelPickerTab::Favorites);
            })
            .child(
                icon(
                    if fav_selected { "star" } else { "star-off" },
                    if fav_selected { p.ink } else { p.muted },
                )
                .size(18.0, 18.0),
            ),
    );

    let mut seen: Vec<(String, String)> = Vec::new();
    for entry in &snap.sidebar_entries {
        if seen.iter().any(|(earlier_id, earlier_name)| {
            earlier_id == &entry.instance_id && earlier_name == &entry.display_name
        }) {
            continue;
        }
        let repeats = seen
            .iter()
            .filter(|(earlier_id, _)| earlier_id == &entry.instance_id)
            .count();
        seen.push((entry.instance_id.clone(), entry.display_name.clone()));
        let key = if repeats == 0 {
            format!("choose-harness-{}", entry.instance_id)
        } else {
            format!("choose-harness-{}-{}", entry.instance_id, repeats + 1)
        };

        let is_unavailable = !is_provider_instance_picker_ready(entry);
        let is_context_disabled = snap.locked_disabled_ids.contains(&entry.instance_id);
        let unavailable_reachable = snap.selectable_unavailable_ids.contains(&entry.instance_id);
        let is_disabled = (is_unavailable && !unavailable_reachable) || is_context_disabled;
        let is_selected = snap.selected_tab == ModelPickerTab::Instance(entry.instance_id.clone());
        let tooltip = if is_unavailable {
            describe_unavailable_instance(entry)
        } else if is_context_disabled {
            format!(
                "{} is unavailable in this thread. Start a new thread to switch providers.",
                entry.display_name
            )
        } else {
            entry.display_name.clone()
        };

        let mut btn_content = col()
            .items_center()
            .justify_center()
            .gap(1.0)
            .opacity(if is_selected { 1.0 } else { 0.7 })
            .child(harness_icon(s, p, &entry.instance_id, &entry.display_name));
        if entry.is_acp {
            btn_content = btn_content.child(outlined(
                meta_text("ACP", p.muted)
                    .key(format!("harness-tag-{}", entry.instance_id))
                    .px(2.0)
                    .radius(4.0),
                p,
            ));
        }

        let mut item = row()
            .key(key)
            .role("button")
            .label(&tooltip)
            .help(&tooltip)
            .checked(is_selected)
            .disabled(is_disabled)
            .opacity(if is_disabled { 0.5 } else { 1.0 })
            .w(36.0)
            .h(if entry.is_acp { 42.0 } else { 36.0 })
            .center()
            .radius(8.0)
            .fill(if is_selected {
                p.selected
            } else {
                Color::TRANSPARENT
            });
        if !is_disabled {
            let target_id = entry.instance_id.clone();
            item = item
                .cursor(rune::Cursor::Pointer)
                .hover(p.selected)
                .on_press(move |s: &mut State| {
                    s.model_picker_tab = Some(ModelPickerTab::Instance(target_id.clone()));
                });
        }
        sidebar = sidebar.child(item.child(btn_content));
    }

    let mut list = col()
        .key("model-list")
        .role("listbox")
        .label("Models")
        .focus_group()
        .w_full()
        .h(0.0)
        .grow(1.0)
        .min_h(0.0)
        .pl(6.0)
        .pr(4.0)
        .py(6.0)
        .gap(2.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        });

    if let Some(error) = &s.harnesses.error {
        list = list.child(label(error, SMALL, p.muted).wrap().px(16.0).py(6.0));
    }

    let multi_enabled = s.multiple_models_enabled();
    let multi_selections = if multi_enabled { selections } else { &[] };

    let rows_mix_providers = snap
        .visible_models
        .iter()
        .any(|m| m.instance_id != snap.visible_models[0].instance_id);
    let mut jump_num = 0usize;
    for (index, model) in snap.visible_models.iter().enumerate() {
        let disabled_reason = if !can_choose {
            Some("Model picker is disabled".to_owned())
        } else {
            s.model_disabled_reason_for(&model.instance_id, &model.slug)
        };
        let enabled = disabled_reason.is_none();
        let jump_label = if enabled && jump_num < 9 {
            jump_num += 1;
            Some(format!("⌘{jump_num}"))
        } else {
            None
        };
        let is_favorite = snap
            .favorites_set
            .contains(&provider_model_key(&model.instance_id, &model.slug));
        let selected = if multi_enabled && multi_selections.len() > 1 {
            multi_selections
                .iter()
                .any(|sel| sel.instance_id == model.instance_id && sel.model == model.slug)
        } else {
            t.harness == model.instance_id && t.model_id == model.slug
        };
        let display_name = get_display_model_name(
            &model.name,
            model.short_name.as_deref(),
            model.sub_provider.as_deref(),
            false,
        );

        // KSG's `ModelListRow`: the name with a check when selected, then its badges.
        let mut name_row = row().center_y().gap(6.0).min_w(0.0).child(
            label(&display_name, TextStyle::new(13.0, 500.0), p.ink)
                .truncate()
                .shrink()
                .min_w(0.0),
        );
        if selected {
            name_row = name_row.child(icon("check", p.ink).size(14.0, 14.0));
        }
        if model.badge.as_deref() == Some("new") {
            name_row = name_row.child(outlined(
                meta_text("New", p.muted)
                    .label("New model")
                    .px(4.0)
                    .py(1.0)
                    .radius(6.0),
                p,
            ));
        }
        if model.is_unavailable {
            name_row = name_row.child(outlined(
                meta_text("Unavailable", p.muted)
                    .px(4.0)
                    .py(1.0)
                    .radius(6.0),
                p,
            ));
        }

        let mut name_col = col().gap(4.0).grow(1.0).min_w(0.0).child(name_row);
        // The provider line, when the rows mix providers or the model names a sub-provider.
        if rows_mix_providers || model.sub_provider.is_some() {
            // KSG `ModelListRow` uses the sub-provider name on this line when present.
            let provider_label = model
                .sub_provider
                .as_deref()
                .unwrap_or(&model.instance_display_name);
            name_col = name_col.child(
                row()
                    .center_y()
                    .gap(6.0)
                    .min_w(0.0)
                    .child(
                        provider_icon(
                            s,
                            p,
                            model_vendor(&model.instance_id, &model.instance_display_name),
                            12.0,
                        )
                        .opacity(0.6),
                    )
                    .child(
                        label(provider_label, META, p.faint)
                            .truncate()
                            .shrink()
                            .min_w(0.0),
                    ),
            );
        }
        // A started thread hands its context to the new provider; the row says so
        // under the name, KSG Code's pending-handoff wording for the pick.
        let handoff_hint = enabled && t.has_started_session() && model.instance_id != t.harness;
        if handoff_hint {
            name_col = name_col.child(
                label("Starts a context handoff", META, p.muted)
                    .key(format!("handoff-hint-{index}"))
                    .truncate()
                    .shrink()
                    .min_w(0.0),
            );
        }

        let fav_provider = model.instance_id.clone();
        let fav_slug = model.slug.clone();
        let mut star_btn = row()
            .key(format!("toggle-favorite-{index}"))
            .role("button")
            .label(if is_favorite {
                "Remove from favorites"
            } else {
                "Add to favorites"
            })
            .help(if is_favorite {
                "Remove from favorites"
            } else {
                "Add to favorites"
            })
            .focusable(false)
            .size(24.0, 24.0)
            .center()
            .radius(6.0)
            .disabled(!enabled)
            .child(
                icon(
                    if is_favorite { "star" } else { "star-off" },
                    if is_favorite { p.brass } else { p.muted },
                )
                .size(14.0, 14.0),
            );
        if enabled {
            star_btn = star_btn
                .cursor(rune::Cursor::Pointer)
                .hover(p.selected)
                .on_press(move |s: &mut State| {
                    s.toggle_model_favorite(&fav_provider, &fav_slug);
                });
        }

        let mut trailing = row().center_y().gap(6.0).shrink().child(star_btn);
        if let Some(jump) = jump_label {
            trailing = trailing.child(keycap_chip(&jump, p));
        }

        // Clicking a row picks the model; Shift adds it to a multi-model send, as KSG's
        // combobox does. There is no checkbox.
        let mut item = row()
            .key(format!("choose-model-{index}"))
            .role("option")
            .label(format!("{display_name}, {}", model.instance_display_name))
            .checked(selected)
            .focusable(enabled)
            .disabled(!enabled)
            .opacity(if enabled { 1.0 } else { 0.55 })
            .center_y()
            .gap(8.0)
            .min_h(32.0)
            .px(8.0)
            .py(5.0)
            .radius(6.0)
            .hover(p.selected)
            .focus_fill(p.selected)
            .focus_selected(selected)
            .child(name_col)
            .child(trailing);
        if let Some(reason) = &disabled_reason {
            item = item.help(reason);
        }
        if enabled {
            let instance_id = model.instance_id.clone();
            let id = model.slug.clone();
            item = item
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(move |s: &mut State, tx| {
                    if tx.mods().shift && s.multiple_models_enabled() {
                        s.toggle_model_in_multi_selection(&instance_id, &id);
                    } else {
                        s.select_model_from_picker(&instance_id, &id);
                        finish_model_pick(s);
                    }
                });
        }
        list = list.child(item);
    }

    if let Some((instance_id, expanded)) = &snap.legacy_section {
        let toggle_id = instance_id.clone();
        list = list.child(
            row()
                .key(format!("legacy-models-{instance_id}"))
                .role("option")
                .label(if *expanded { "Show less" } else { "Show more" })
                .focusable(true)
                .center_y()
                .h(32.0)
                .mx(4.0)
                .px(12.0)
                .radius(6.0)
                .hover(p.selected)
                .focus_fill(p.selected)
                .cursor(rune::Cursor::Pointer)
                .on_press(move |s: &mut State| {
                    if !s.expanded_legacy_instances.remove(&toggle_id) {
                        s.expanded_legacy_instances.insert(toggle_id.clone());
                    }
                })
                .child(label(
                    if *expanded { "Show less" } else { "Show more" },
                    TextStyle::new(13.0, 500.0),
                    p.muted,
                )),
        );
    }

    let selected_id = match &snap.selected_tab {
        ModelPickerTab::Instance(id) => Some(id.as_str()),
        ModelPickerTab::Favorites => None,
    };
    let active_harness = selected_id.unwrap_or(&t.harness);
    let (pi_loading, pi_error) = if active_harness == "pi" {
        s.pi_catalog_status()
    } else {
        (false, None)
    };
    let (native_loading, native_error) = s.native_catalog_status(active_harness);
    let catalog_loading =
        pi_loading || (matches!(active_harness, "codex" | "claude") && native_loading);
    let catalog_error = pi_error.or(native_error);

    if snap.visible_models.is_empty() && snap.setup_entries.is_empty() {
        let empty_text = if snap.is_searching {
            "No models found"
        } else if snap.selected_tab == ModelPickerTab::Favorites {
            "No favorites yet"
        } else if active_harness == "claude"
            && snap
                .entries
                .iter()
                .find(|e| e.instance_id == "claude")
                .is_some_and(|e| !e.installed)
        {
            crate::claude::MISSING
        } else if matches!(active_harness, "codex" | "claude") {
            if native_loading {
                "Reading the harness catalog..."
            } else if let Some(err) = native_error {
                err
            } else {
                "Catalog not available. Models will be read when a session starts."
            }
        } else if active_harness != "pi" {
            "This harness uses its configured model until it reports available models."
        } else if let Some(error) = pi_error {
            error
        } else if s.catalog.remaining().is_some() {
            "Reading the CPA catalog..."
        } else {
            "No CPA models are available."
        };
        let note_key = if active_harness == "claude"
            && snap
                .entries
                .iter()
                .find(|e| e.instance_id == "claude")
                .is_some_and(|e| !e.installed)
        {
            "claude-unavailable"
        } else {
            "models-empty-note"
        };
        list = list
            .child(rule(p).mt(6.0))
            .child(
                meta_label("Models", p.muted)
                    .key("models-group")
                    .px(16.0)
                    .pt(8.0)
                    .pb(4.0),
            )
            .child(
                label(
                    empty_text,
                    SMALL,
                    if catalog_error.is_some() {
                        p.danger
                    } else {
                        p.muted
                    },
                )
                .key(note_key)
                .wrap()
                .px(16.0)
                .py(10.0),
            );
        if catalog_error.is_some() {
            list = list.child(
                button("retry-catalog", "Retry", p, |s| s.force_refresh_catalog())
                    .ml(12.0)
                    .mb(8.0),
            );
        }
    }

    let mut right_col = col()
        .w(0.0)
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .min_h(0.0)
        .child(header)
        .child(rule(p))
        .child(list);

    if catalog_loading && !snap.visible_models.is_empty() {
        right_col = right_col.child(rule(p)).child(
            label("Refreshing models...", SMALL, p.muted)
                .key("catalog-loading")
                .px(12.0)
                .py(6.0),
        );
    }
    if let Some(error) = catalog_error
        && !snap.visible_models.is_empty()
    {
        right_col = right_col.child(rule(p)).child(
            col()
                .gap(4.0)
                .px(12.0)
                .py(6.0)
                .child(label(error, SMALL, p.danger).wrap_anywhere())
                .child(button("retry-catalog", "Retry", p, |s| {
                    s.force_refresh_catalog()
                })),
        );
    }

    if !snap.setup_entries.is_empty() {
        let mut setup_box = col()
            .gap(6.0)
            .px(12.0)
            .py(8.0)
            .border(0.0, Color::TRANSPARENT);
        let multi = snap.setup_entries.len() > 1;
        for entry in &snap.setup_entries {
            let btn_label = if multi {
                format!("Set up {}", entry.display_name)
            } else {
                "Open provider setup".to_owned()
            };
            setup_box = setup_box.child(
                col()
                    .gap(4.0)
                    .child(label(get_provider_status_message(entry), SMALL, p.muted).wrap())
                    .child(button(
                        &format!("open-provider-setup-{}", entry.instance_id),
                        &btn_label,
                        p,
                        |s| {
                            s.settings_picker = None;
                            s.open_settings();
                            s.settings.section = 2;
                        },
                    )),
            );
        }
        right_col = right_col.child(rule(p)).child(setup_box);
    }

    for (instance_id, notice) in &snap.update_required_notices {
        right_col = right_col.child(rule(p)).child(
            col()
                .key(format!("update-required-notice-{instance_id}"))
                .gap(4.0)
                .px(12.0)
                .py(8.0)
                .child(label(notice, SMALL, p.muted).wrap())
                .child(button(
                    &format!("provider-settings-{instance_id}"),
                    "Provider settings",
                    p,
                    |s| {
                        s.settings_picker = None;
                        s.open_settings();
                        s.settings.section = 2;
                    },
                )),
        );
    }

    let mut body = row().w_full().h_full().min_w(0.0).min_h(0.0);
    if show_sidebar {
        body = body.child(sidebar);
    }
    body = body.child(right_col);
    card.child(body)
}

/// A harness's display name: "Pi", "Codex", "Claude" for the built-ins, the registry's
/// configured name for ACP agents (allowing overrides of "antigravity"), built-in
/// "Antigravity" when unregistered, or a generic "agent" when the id is unknown.
pub(super) fn harness_name<'a>(s: &'a State, id: &'a str) -> &'a str {
    match id {
        "pi" => "Pi",
        "codex" => "Codex",
        "claude" => "Claude",
        id => s
            .harnesses
            .agents
            .iter()
            .find(|agent| agent.id == id)
            .map(|agent| agent.name.as_str())
            .unwrap_or(match id {
                "antigravity" => "Antigravity",
                _ => "agent",
            }),
    }
}
/// A harness row's mark, as KSG Code's `ProviderInstanceIcon` draws an ACP agent: the vendor's
/// mark when the agent names one, Pi's own glyph, or a generic agent glyph.
pub(super) fn harness_icon(s: &State, p: Palette, id: &str, name: &str) -> El {
    static PI: Icon = Icon {
        name: "PI",
        view: 16.0,
        body: r#"<path d="M3 4.5h10M6 4.5V12M10.25 4.5v5.75a1.75 1.75 0 0 0 1.75 1.75h.5" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"/>"#,
    };
    static AGENT: Icon = Icon {
        name: "AGENT",
        view: 16.0,
        body: r#"<rect x="2" y="3" width="12" height="10" rx="2.5" fill="none" stroke="currentColor"/><path d="m5 6.5 2 1.5-2 1.5M8.5 10H11" fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round"/>"#,
    };
    if id == "pi" {
        return rune::icon(&PI, 16.0).color(p.ink).a11y_hidden();
    }
    match model_vendor(id, name) {
        Vendor::Other => rune::icon(&AGENT, 16.0).color(p.muted).a11y_hidden(),
        vendor => provider_icon(s, p, vendor, 16.0),
    }
}

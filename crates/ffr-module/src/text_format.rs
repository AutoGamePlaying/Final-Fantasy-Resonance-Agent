//! Shared Simplified-Chinese (`zh-Hans`) text resolution and untranslated-entry formatting.
//!
//! Distinguishes three states for any localized `FTextValue`:
//! 1. [`ZhText::Translated`]: genuinely translated `"zh-Hans"` string.
//! 2. [`ZhText::Untranslated`]: entry present in the game's `"zh-Hans"` localization file, but
//!    left verbatim as Japanese source text (recorded in `text_untranslated` / `*_untranslated`).
//! 3. [`ZhText::Missing`]: absent from `"zh-Hans"` localization or empty.
//!
//! Model-facing text summaries render [`ZhText::Untranslated`] explicitly as
//! `（游戏中文版未翻译，日文原文：…）` without ever passing Japanese off as Chinese or silently
//! dropping it as blank. Structured JSON keeps `<field>: null` alongside `<stem>_untranslated: true`
//! and `<stem>_ja: "…"`.

use ffr_data::{
    FTextValue, MODEL_LANGUAGE, ResolveOptions, ResolvedText, TextResolver, UNTRANSLATED_REASON,
};
use ffr_db::GameDb;
use ffr_db::query::links::MapUnit;
use serde_json::{Map, Value, json};

use crate::data_tools::collapse_ws;

/// Resolution state of one localized `"zh-Hans"` field for model-facing output.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) enum ZhText {
    /// Genuinely translated Simplified Chinese text.
    Translated(String),
    /// Official `"zh-Hans"` localization entry left untranslated as Japanese source text.
    Untranslated { ja: String },
    /// No localization entry (or whitespace-only / null `CultureInvariantString`).
    #[default]
    Missing,
}

impl ZhText {
    /// Build a [`ZhText`] from a [`ResolvedText`] produced with `ResolveOptions::with_fallback()`,
    /// preserving raw whitespace.
    pub(crate) fn from_resolved_raw(res: &ResolvedText) -> Self {
        let Some(text) = res.text.as_ref() else {
            return Self::Missing;
        };
        if !res.is_fallback() {
            return Self::Translated(text.clone());
        }
        if res
            .reason
            .as_deref()
            .is_some_and(|reason| reason.starts_with(UNTRANSLATED_REASON))
        {
            return Self::Untranslated { ja: text.clone() };
        }
        Self::Missing
    }

    /// Build a [`ZhText`] from a [`ResolvedText`] produced with `ResolveOptions::with_fallback()`,
    /// collapsing runs of whitespace to single spaces.
    pub(crate) fn from_resolved_clean(res: &ResolvedText) -> Self {
        let Some(text) = res.text.as_deref().and_then(collapse_ws) else {
            return Self::Missing;
        };
        if !res.is_fallback() {
            return Self::Translated(text);
        }
        if res
            .reason
            .as_deref()
            .is_some_and(|reason| reason.starts_with(UNTRANSLATED_REASON))
        {
            return Self::Untranslated { ja: text };
        }
        Self::Missing
    }

    /// Build a [`ZhText`] from an optional translated Chinese string and an optional raw
    /// untranslated Japanese value, collapsing whitespace.
    pub(crate) fn from_parts_clean(zh: Option<&str>, untranslated_ja: Option<&str>) -> Self {
        if let Some(clean_zh) = zh.and_then(collapse_ws) {
            return Self::Translated(clean_zh);
        }
        if let Some(clean_ja) = untranslated_ja.and_then(collapse_ws) {
            return Self::Untranslated { ja: clean_ja };
        }
        Self::Missing
    }

    /// Return the genuinely translated Chinese string, or `None` when untranslated or missing.
    pub(crate) fn zh(&self) -> Option<&str> {
        match self {
            Self::Translated(text) => Some(text.as_str()),
            Self::Untranslated { .. } | Self::Missing => None,
        }
    }

    /// Return the Japanese original string when officially untranslated, otherwise `None`.
    pub(crate) fn untranslated_ja(&self) -> Option<&str> {
        match self {
            Self::Untranslated { ja } => Some(ja.as_str()),
            Self::Translated(_) | Self::Missing => None,
        }
    }

    /// Return `true` when this field is officially untranslated in `"zh-Hans"`.
    pub(crate) fn is_untranslated(&self) -> bool {
        matches!(self, Self::Untranslated { .. })
    }

    /// Collapse whitespace inside a raw [`ZhText`] (useful when a raw resolution is also needed in
    /// collapsed form).
    pub(crate) fn cleaned(&self) -> Self {
        match self {
            Self::Translated(s) => collapse_ws(s).map_or(Self::Missing, Self::Translated),
            Self::Untranslated { ja } => {
                collapse_ws(ja).map_or(Self::Missing, |ja| Self::Untranslated { ja })
            }
            Self::Missing => Self::Missing,
        }
    }

    /// Standard Chinese phrasing for an officially untranslated entry.
    pub(crate) fn format_untranslated(ja: &str) -> String {
        format!("（游戏中文版未翻译，日文原文：{ja}）")
    }

    /// Render as `「zh」`, `（游戏中文版未翻译，日文原文：ja）`, or `「missing_placeholder」`.
    pub(crate) fn bracketed_or(&self, missing_placeholder: &str) -> String {
        match self {
            Self::Translated(zh) => format!("「{zh}」"),
            Self::Untranslated { ja } => Self::format_untranslated(ja),
            Self::Missing => format!("「{missing_placeholder}」"),
        }
    }

    /// Render as `「zh」`, `（游戏中文版未翻译，日文原文：ja）`, or `（missing_in_parens）`.
    pub(crate) fn bracketed_or_paren(&self, missing_in_parens: &str) -> String {
        match self {
            Self::Translated(zh) => format!("「{zh}」"),
            Self::Untranslated { ja } => Self::format_untranslated(ja),
            Self::Missing => format!("（{missing_in_parens}）"),
        }
    }

    /// Render as `「zh」`, `（游戏中文版未翻译，日文原文：ja）`, or empty string when missing.
    pub(crate) fn bracketed_or_empty(&self) -> String {
        match self {
            Self::Translated(zh) => format!("「{zh}」"),
            Self::Untranslated { ja } => Self::format_untranslated(ja),
            Self::Missing => String::new(),
        }
    }

    /// Render inline text: `zh`, `（游戏中文版未翻译，日文原文：ja）`, or `missing_placeholder`.
    pub(crate) fn inline_or(&self, missing_placeholder: &str) -> String {
        match self {
            Self::Translated(zh) => zh.clone(),
            Self::Untranslated { ja } => Self::format_untranslated(ja),
            Self::Missing => missing_placeholder.to_owned(),
        }
    }

    /// Render optional inline text: `Some(zh)`, `Some("（游戏中文版未翻译，日文原文：ja）")`, or `None`.
    pub(crate) fn inline_opt(&self) -> Option<String> {
        match self {
            Self::Translated(zh) => Some(zh.clone()),
            Self::Untranslated { ja } => Some(Self::format_untranslated(ja)),
            Self::Missing => None,
        }
    }

    /// Write `<field_name>` (and, when untranslated, `<stem>_untranslated: true` and
    /// `<stem>_ja: "<ja>"`) into `obj`.
    ///
    /// When `field_name` ends with `_zh` (e.g. `name_zh`, `npc_name_zh`), `<stem>` strips the
    /// `_zh` suffix so companion keys are `name_untranslated` / `name_ja` (or
    /// `npc_name_untranslated` / `npc_name_ja`).
    pub(crate) fn insert_json_fields(&self, obj: &mut Map<String, Value>, field_name: &str) {
        obj.insert(field_name.to_owned(), json!(self.zh()));
        if let Some(ja) = self.untranslated_ja() {
            let stem = field_name.strip_suffix("_zh").unwrap_or(field_name);
            obj.insert(format!("{stem}_untranslated"), Value::Bool(true));
            obj.insert(format!("{stem}_ja"), Value::String(ja.to_owned()));
        }
    }
}

/// Attach one or more [`ZhText`] fields onto a JSON object [`Value`].
pub(crate) fn with_zh_fields(mut value: Value, fields: &[(&str, &ZhText)]) -> Value {
    if let Some(obj) = value.as_object_mut() {
        for (field_name, resolved) in fields {
            resolved.insert_json_fields(obj, field_name);
        }
    }
    value
}

/// Resolve an [`FTextValue`] in `"zh-Hans"` through [`TextResolver`], collapsing whitespace and
/// distinguishing official untranslated entries from truly missing keys.
pub(crate) fn resolve_zh_text_clean(resolver: &TextResolver, value: &FTextValue) -> ZhText {
    let resolved =
        resolver.resolve_with(value, Some(MODEL_LANGUAGE), ResolveOptions::with_fallback());
    ZhText::from_resolved_clean(&resolved)
}

/// Resolve an [`FTextValue`] in `"zh-Hans"` through [`TextResolver`], preserving raw whitespace.
pub(crate) fn resolve_zh_text_raw(resolver: &TextResolver, value: &FTextValue) -> ZhText {
    let resolved =
        resolver.resolve_with(value, Some(MODEL_LANGUAGE), ResolveOptions::with_fallback());
    ZhText::from_resolved_raw(&resolved)
}

/// Resolve an [`FTextValue`] in `"zh-Hans"` through [`GameDb`], collapsing whitespace and
/// distinguishing official untranslated entries from truly missing keys.
pub(crate) fn resolve_db_zh_text_clean(db: &GameDb, value: &FTextValue) -> ZhText {
    match db.resolve_ftext(value, MODEL_LANGUAGE, ResolveOptions::with_fallback()) {
        Ok(resolved) => ZhText::from_resolved_clean(&resolved),
        Err(_) => ZhText::Missing,
    }
}

/// Resolve an [`FTextValue`] in `"zh-Hans"` through [`GameDb`], preserving raw whitespace.
pub(crate) fn resolve_db_zh_text_raw(db: &GameDb, value: &FTextValue) -> ZhText {
    match db.resolve_ftext(value, MODEL_LANGUAGE, ResolveOptions::with_fallback()) {
        Ok(resolved) => ZhText::from_resolved_raw(&resolved),
        Err(_) => ZhText::Missing,
    }
}

/// Resolve a pre-joined `(zh_text, text_ns, text_key)` triple against `GameDb`'s
/// `text_untranslated` table when `zh_text` is `None`.
pub(crate) fn resolve_db_text_key_clean(
    db: &GameDb,
    zh_text: Option<&str>,
    ns: Option<&str>,
    key: Option<&str>,
) -> ZhText {
    if let Some(clean_zh) = zh_text.and_then(collapse_ws) {
        return ZhText::Translated(clean_zh);
    }
    if let (Some(ns), Some(key)) = (ns, key)
        && let Ok(Some(entry)) = db.text_untranslated(ns, key, MODEL_LANGUAGE)
        && let Some(clean_ja) = collapse_ws(&entry.value)
    {
        return ZhText::Untranslated { ja: clean_ja };
    }
    ZhText::Missing
}

/// Resolve a [`MapUnit`]'s `"zh-Hans"` name, checking `text_untranslated` / `map_unit_untranslated`
/// when `name_zh` is `None`.
pub(crate) fn resolve_db_map_unit_name_zh(
    db: &GameDb,
    name_zh: Option<&str>,
    map_unit: Option<&MapUnit>,
    map_unit_id: Option<i64>,
) -> ZhText {
    if let Some(clean_zh) = name_zh.and_then(collapse_ws) {
        return ZhText::Translated(clean_zh);
    }
    if let Some(mu) = map_unit {
        let from_key = resolve_db_text_key_clean(
            db,
            mu.name_zh.as_deref(),
            mu.name_ns.as_deref(),
            mu.name_key.as_deref(),
        );
        if !matches!(from_key, ZhText::Missing) {
            return from_key;
        }
    }
    if let Some(id) = map_unit_id
        && let Ok(fields) = db.map_unit_untranslated_fields(id)
        && let Some(hit) = fields
            .into_iter()
            .find(|f| f.field == "name" && f.lang == MODEL_LANGUAGE)
        && let Some(clean_ja) = collapse_ws(&hit.value)
    {
        return ZhText::Untranslated { ja: clean_ja };
    }
    ZhText::Missing
}

/// Format an item reference for inline summaries (`清露草药（#1）`,
/// `道具 #10699（游戏中文版未翻译，日文原文：オールドウィップ）`, or `道具 #9500`).
pub(crate) fn format_item_ref_zh(item_id: i64, name: &ZhText) -> String {
    match name {
        ZhText::Translated(zh) => format!("{zh}（#{item_id}）"),
        ZhText::Untranslated { ja } => {
            format!("道具 #{item_id}{}", ZhText::format_untranslated(ja))
        }
        ZhText::Missing => format!("道具 #{item_id}"),
    }
}

/// Format a map reference for inline summaries (`地图 #3001「微光洞窟」`,
/// `地图 #3001（游戏中文版未翻译，日文原文：…）`, or `地图 #3001`).
pub(crate) fn format_map_ref_zh(map_id: i64, name: &ZhText) -> String {
    format!("地图 #{map_id}{}", name.bracketed_or_empty())
}

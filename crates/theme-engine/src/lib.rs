//! CSS subset engine for Rustsidian themes and snippets.
//!
//! Supported: type/class/id/attribute selectors, `:hover :focus :active :disabled :checked
//! :first-child :last-child :root :not() :is() :where()`, all four combinators, specificity,
//! `!important`, custom properties with `var()` fallbacks, `calc()/min()/max()/clamp()`,
//! `@media (prefers-color-scheme)` and `(prefers-reduced-motion)`, and the `margin`, `padding`,
//! `border*`, `background`, `gap`, `overflow` shorthands.
//! Not supported: pseudo-elements, `:nth-child()`, `:has()`, `@supports`, `@layer`, `@container`,
//! width media queries, `@font-face`, `@keyframes`, gradients and background images.

pub mod cascade;
pub mod css;
pub mod selector;
pub mod value;

pub use cascade::{Computed, Env, MatchedRule, Theme};
pub use selector::Element;
pub use value::{parse_color, parse_length, Color, Easing, LengthCx, Transition};

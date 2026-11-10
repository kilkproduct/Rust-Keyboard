//! The suggestion search.
//!
//! This is a re-implementation of the typed/gesture search from
//! `suggest/core/suggest.cpp`, keeping the scoring model of the original
//! (`scoring_params.cpp` / `typing_scoring.h`) so that ranking, and therefore
//! the visible behaviour of the keyboard, stays the same.
//!
//! Differences from the C++ implementation, all deliberate:
//!
//! * the node cache is a fixed-size array instead of a priority queue with
//!   per-node heap allocations,
//! * multi-word suggestions are produced from the committed previous words
//!   (the usual case) rather than by splitting the current word,
//! * digraph expansion (German "ue" -> "ü") is not implemented yet,
//! * the gesture decoder matches the swipe path as a subsequence of touch
//!   points instead of using the full alignment model.

use crate::charutils;
use crate::dict::{Dictionary, WordAttributes};
use crate::proximity::{ProximityInfo, ProximityType, NOT_AN_INDEX};
use crate::ptrie::Node;
use crate::user;
use crate::{
    KIND_CORRECTION, KIND_FLAG_APPROPRIATE_FOR_AUTOCORRECTION, KIND_FLAG_EXACT_MATCH,
    KIND_FLAG_POSSIBLY_OFFENSIVE, KIND_TYPED, MAX_PROXIMITY_CHARS_SIZE, MAX_RESULTS,
    MAX_WORD_LENGTH, NOT_A_PROBABILITY,
};

// Scoring constants, copied from `scoring_params.cpp` and `defines.h`.
const DISTANCE_WEIGHT_LENGTH: f32 = 0.1524;
const PROXIMITY_COST: f32 = 0.0694;
const OMISSION_COST: f32 = 0.467;
const INSERTION_COST: f32 = 0.7248;
const SUBSTITUTION_COST: f32 = 0.3806;
const COST_FIRST_COMPLETION: f32 = 0.4836;
const COST_COMPLETION: f32 = 0.00624;
const DISTANCE_WEIGHT_LANGUAGE: f32 = 1.1214;
const TYPING_BASE_OUTPUT_SCORE: f32 = 1.0;
const TYPING_MAX_OUTPUT_SCORE_PER_INPUT: f32 = 0.1;
const EXACT_MATCH_PROMOTION: f32 = 1.1;
const SUGGEST_INTERFACE_OUTPUT_SCALE: f32 = 1_000_000.0;
const MAX_PROBABILITY_FOR_WEIGHTING: f32 = 255.0;

// Gesture-only penalties.
const GESTURE_SKIP_COST: f32 = 0.16;
const GESTURE_UNUSED_POINT_COST: f32 = 0.22;
const GESTURE_MAX_POINT_LOOKAHEAD: usize = 6;

// Search bounds. Small on purpose: a keyboard must never grow with typing.
const MAX_STATES: usize = 192;
const MAX_TERMINALS: usize = 96;
const MAX_EXTRA_COMPLETION_STEPS: usize = 8;
/// Frontier size that triggers an early prune; keeps the working set small.
/// Bound on how many partial matches the search keeps at once. Every entry is a
/// whole `Node` (word characters included), so this is the engine's largest
/// transient allocation; the frontier is trimmed to the best scores anyway.
const MAX_FRONTIER: usize = 256;

/// Error flags accumulated along a search path.
pub const ERROR_EDIT: u32 = 0x1;
pub const ERROR_PROXIMITY: u32 = 0x2;
pub const ERROR_COMPLETION: u32 = 0x4;
pub const ERROR_CASE: u32 = 0x8;
pub const ERROR_ACCENT: u32 = 0x10;

/// Mirrors `NativeSuggestOptions` on the Java side.
#[derive(Clone, Copy)]
pub struct SuggestOptions {
    pub is_gesture: bool,
    pub use_full_edit_distance: bool,
    pub block_offensive_words: bool,
    pub space_aware_gesture_enabled: bool,
    pub weight_for_locale: f32,
}

impl SuggestOptions {
    pub fn defaults() -> SuggestOptions {
        SuggestOptions {
            is_gesture: false,
            use_full_edit_distance: false,
            block_offensive_words: false,
            space_aware_gesture_enabled: false,
            weight_for_locale: 1.0f32,
        }
    }

    /// `options[0]` is gesture, `[1]` full edit distance, `[2]` block offensive,
    /// `[3]` space aware gesture, `[4]` locale weight in thousandths.
    pub fn from_array(options: &[i32]) -> SuggestOptions {
        let mut result = SuggestOptions::defaults();
        if let Some(value) = options.first() {
            result.is_gesture = *value != 0;
        }
        if let Some(value) = options.get(1) {
            result.use_full_edit_distance = *value != 0;
        }
        if let Some(value) = options.get(2) {
            result.block_offensive_words = *value != 0;
        }
        if let Some(value) = options.get(3) {
            result.space_aware_gesture_enabled = *value != 0;
        }
        if let Some(value) = options.get(4) {
            result.weight_for_locale = *value as f32 / 1000.0f32;
        }
        result
    }
}

/// Node visits per prediction in the trie-walk fallback.
const PREDICTION_NODE_BUDGET: usize = 1200;
/// Ceiling on the breadth-first queue of the fallback walk: 1200 entries of
/// fixed-size prefixes is ~230 KB, where the old 16000 Vec-carrying entries
/// could grow to megabytes on a corrupt trie.
const PREDICTION_QUEUE_LIMIT: usize = 1200;

/// A word prefix carried through the fallback walk, in place.
#[derive(Clone, Copy)]
struct Prefix {
    code_points: [i32; MAX_WORD_LENGTH],
    len: u8,
}

impl Prefix {
    fn empty() -> Prefix {
        Prefix {
            code_points: [0; MAX_WORD_LENGTH],
            len: 0,
        }
    }

    fn push_node(&mut self, node: &Node) {
        for index in 0..node.char_count as usize {
            if (self.len as usize) >= MAX_WORD_LENGTH {
                return;
            }
            self.code_points[self.len as usize] = node.char_at(index);
            self.len += 1;
        }
    }

    fn to_suggestion(&self) -> SuggestedWord {
        let mut suggestion = SuggestedWord::empty();
        let length = self.len as usize;
        suggestion.code_points[..length].copy_from_slice(&self.code_points[..length]);
        suggestion.length = self.len;
        suggestion
    }
}

/// One suggestion, laid out exactly like the arrays the Java side reads:
/// `MAX_WORD_LENGTH` code points per slot plus a score and an attribute mask.
#[derive(Clone, Copy)]
pub struct SuggestedWord {
    pub code_points: [i32; MAX_WORD_LENGTH],
    pub length: u8,
    pub score: i32,
    pub kind: i32,
    pub index_to_partial_commit: i32,
    pub auto_commit_first_word_confidence: i32,
}

impl SuggestedWord {
    pub fn empty() -> SuggestedWord {
        SuggestedWord {
            code_points: [0; MAX_WORD_LENGTH],
            length: 0,
            score: 0,
            kind: 0,
            index_to_partial_commit: 0,
            auto_commit_first_word_confidence: 0,
        }
    }

    pub fn as_slice(&self) -> &[i32] {
        &self.code_points[..self.length as usize]
    }
}

#[derive(Clone, Copy)]
struct InputState {
    primary: i32,
    x: i32,
    y: i32,
    // Kept for the key-proximity pass that filled them; the scoring above only
    // uses the primary key today.
    #[allow(dead_code)]
    proximity: [i32; MAX_PROXIMITY_CHARS_SIZE],
    #[allow(dead_code)]
    proximity_len: u8,
}

#[derive(Clone, Copy)]
struct State {
    node: Node,
    input_index: u16,
    spatial: f32,
    word: [i32; MAX_WORD_LENGTH],
    word_len: u8,
    errors: u32,
    /// How many edits (insert/substitute/omit) this path has paid for.
    edit_count: u8,
    terminal_value: i32,
    is_root: bool,
}

impl State {
    fn root() -> State {
        State {
            node: Node {
                pos: 0,
                children_pos: 0,
                ..Node::empty()
            },
            input_index: 0,
            spatial: 0.0f32,
            word: [0; MAX_WORD_LENGTH],
            word_len: 0,
            errors: 0,
            edit_count: 0,
            terminal_value: NOT_A_PROBABILITY,
            is_root: true,
        }
    }

    fn word_slice(&self) -> &[i32] {
        &self.word[..self.word_len as usize]
    }

    /// Total cost used for pruning and ordering.
    fn cost(&self) -> f32 {
        self.spatial + (self.input_index as f32) * 0.0001f32
    }
}

struct Search<'a> {
    dict: &'a Dictionary,
    proximity: &'a ProximityInfo,
    input: &'a [InputState],
    options: &'a SuggestOptions,
    prev_terminal_ids: &'a [i32],
    weight_of_lang_model_vs_spatial: f32,
    results: &'a mut Vec<SuggestedWord>,
    /// Reusable buffer of the children of the node being expanded. It has to
    /// hold *every* child: a fixed 24 entry window silently dropped whole
    /// branches (the root alone has one per letter and digit), which is how
    /// lowercase input ended up matching only the capital 'I' words.
    children: Vec<Node>,
}

impl<'a> Search<'a> {
    /// Fills `self.children` with every child of `node`, keeping the buffer for
    /// the next expansion so nothing is allocated per node.
    fn collect_children(&mut self, node: &Node) {
        let mut children = std::mem::take(&mut self.children);
        children.clear();
        let dict = self.dict;
        dict.for_each_child(node, |child| {
            if child.char_count > 0 {
                children.push(*child);
            }
            true
        });
        self.children = children;
    }

    /// How many edits a word may differ by. Short input gets one, because two
    /// edits on three letters reaches every short word in the dictionary.
    fn max_edits(&self) -> u8 {
        if self.input.len() <= 4 {
            1
        } else {
            2
        }
    }

    fn allows_error_corrections(&self, state: &State) -> bool {
        if self.options.is_gesture {
            // Gestures have no "typed" characters to compare against.
            return false;
        }
        if state.edit_count >= self.max_edits() {
            return false;
        }
        if state.is_root {
            return true;
        }
        state.spatial * DISTANCE_WEIGHT_LENGTH < 0.9f32
    }

    /// A correction that changes the first letter is not a correction, it is a
    /// different word; offering those is what made the list look random.
    fn keeps_first_letter(&self, state: &State) -> bool {
        if state.errors & ERROR_EDIT == 0 {
            return true;
        }
        let typed = match self.input.first() {
            Some(value) => charutils::to_lower(value.primary),
            None => return true,
        };
        if state.word_len == 0 {
            return false;
        }
        charutils::to_lower(state.word[0]) == typed
    }

    fn record_terminal(&mut self, state: &State) {
        if state.word_len == 0 {
            return;
        }
        if state.terminal_value == NOT_A_PROBABILITY {
            return;
        }

        let attributes: WordAttributes = if self.dict.is_v4() {
            self.dict
                .word_attributes(self.prev_terminal_ids, state.terminal_value)
        } else {
            self.dict
                .word_attributes(&[], state.terminal_value)
        };

        let user_probability = match self.dict.user.as_ref() {
            Some(user_dict) => user_dict.lookup_any_context(state.word_slice()),
            None => None,
        };

        let mut probability = attributes.probability;
        if let Some(value) = user_probability {
            if value > probability {
                probability = value;
            }
        }
        if probability == NOT_A_PROBABILITY {
            // Not in the dictionary as a valid entry, but it may still be a
            // word the user taught us.
            match user_probability {
                Some(value) => probability = value,
                None => return,
            }
        }

        let flags = match self.dict.user.as_ref() {
            Some(user_dict) => user_dict.word_flags(state.word_slice()),
            None => 0u8,
        };
        let is_not_a_word = attributes.is_not_a_word || flags & user::FLAG_NOT_A_WORD != 0;
        let is_offensive = attributes.is_possibly_offensive
            || flags & user::FLAG_POSSIBLY_OFFENSIVE != 0;
        let is_blacklisted = attributes.is_blacklisted;

        if is_not_a_word || is_blacklisted {
            return;
        }
        if !self.options.is_gesture && !self.keeps_first_letter(state) {
            return;
        }
        let exact = state.errors == 0;
        if is_offensive && self.options.block_offensive_words && !exact {
            return;
        }

        // Language model term: how unlikely this word is, in log10 units.
        let clamped = (probability as f32).max(1.0f32);
        let language_term = -(clamped / MAX_PROBABILITY_FOR_WEIGHTING).log10();
        let compound_distance = state.spatial * DISTANCE_WEIGHT_LENGTH
            + language_term * self.weight_of_lang_model_vs_spatial;

        let max_distance = DISTANCE_WEIGHT_LANGUAGE
            + (self.input.len() as f32) * TYPING_MAX_OUTPUT_SCORE_PER_INPUT;
        let mut score = TYPING_BASE_OUTPUT_SCORE - compound_distance / max_distance;
        if exact {
            score += EXACT_MATCH_PROMOTION;
        }
        if self.options.is_gesture && state.input_index as usize + 1 < self.input.len() {
            // The finger went further than the candidate word needs.
            let unused = self.input.len() - state.input_index as usize - 1;
            score -= (unused as f32) * GESTURE_UNUSED_POINT_COST / max_distance;
        }

        let mut kind = if exact { KIND_TYPED } else { KIND_CORRECTION };
        if exact {
            kind |= KIND_FLAG_EXACT_MATCH;
        }
        if !self.options.is_gesture {
            kind |= KIND_FLAG_APPROPRIATE_FOR_AUTOCORRECTION;
        }
        if is_offensive {
            kind |= KIND_FLAG_POSSIBLY_OFFENSIVE;
        }

        let scaled_score = (score * SUGGEST_INTERFACE_OUTPUT_SCALE) as i32;
        let confidence = if exact && !self.options.is_gesture {
            scaled_score
        } else {
            0
        };

        add_result(
            self.results,
            state.word_slice(),
            scaled_score,
            kind,
            state.word_len as i32,
            confidence,
        );
    }

    /// Pushes every successor of `state` into `next`.
    fn expand(&mut self, state: &State, next: &mut Vec<State>) {
        if state.word_len as usize >= MAX_WORD_LENGTH {
            return;
        }
        let has_input = (state.input_index as usize) < self.input.len();

        if !has_input {
            // Everything typed is consumed: keep completing the word.
            self.expand_completion(state, next);
            return;
        }

        let input_position = state.input_index as usize;
        let input_state = self.input[input_position];

        self.collect_children(&state.node);
        let child_count = self.children.len();

        for index in 0..child_count {
            let child = self.children[index];
            let char_count = child.char_count as usize;
            if char_count == 0 {
                continue;
            }
            if state.word_len as usize + char_count > MAX_WORD_LENGTH {
                continue;
            }

            // 1. Exact / proximity match of the whole character group.
            if self.group_matches_input(&child, input_position) {
                let first = charutils::to_lower(child.char_at(0));
                let primary = charutils::to_lower(input_state.primary);
                let base_first = charutils::to_base_lower(child.char_at(0));
                let mut cost = self.proximity.distance_for_code_point(
                    child.char_at(0),
                    input_state.x,
                    input_state.y,
                );
                let mut matched_errors = 0u32;
                if first == primary {
                    let proximity_type = self.proximity.proximity_type(
                        input_state.primary,
                        child.char_at(0),
                        input_state.x,
                        input_state.y,
                    );
                    if proximity_type == ProximityType::ProximityChar
                        && self.proximity.key_index_of(child.char_at(0)) != NOT_AN_INDEX
                    {
                        matched_errors |= ERROR_PROXIMITY;
                        cost += PROXIMITY_COST;
                    }
                } else if base_first == charutils::to_base_lower(input_state.primary) {
                    // Only the accent differs: still an "exact" match for the
                    // purposes of autocorrection, with a small penalty.
                    matched_errors |= ERROR_ACCENT;
                    cost += 0.02f32;
                } else {
                    // Only the case differs.
                    matched_errors |= ERROR_CASE;
                    cost += 0.01f32;
                }

                let new_state =
                    self.make_child_state(state, child, cost, matched_errors, char_count);
                next.push(new_state);
                continue;
            }

            if !self.allows_error_corrections(state) {
                continue;
            }

            // 2. Substitution: the dictionary character is not what was typed.
            if char_count == 1 {
                let substitution_cost = SUBSTITUTION_COST
                    + self.proximity.distance_for_code_point(
                        input_state.primary,
                        input_state.x,
                        input_state.y,
                    );
                let substituted =
                    self.make_child_state(state, child, substitution_cost, ERROR_EDIT, 1);
                next.push(substituted);
            }

            // 3. Omission: a dictionary character the user did not type.
            let omitted = self.make_child_state(state, child, OMISSION_COST, ERROR_EDIT, 0);
            next.push(omitted);
        }

        // 4. Insertion: a typed character the dictionary word does not have.
        if self.allows_error_corrections(state) {
            let mut insertion = *state;
            insertion.input_index = state.input_index + 1;
            insertion.spatial += INSERTION_COST;
            insertion.errors |= ERROR_EDIT;
            insertion.edit_count = state.edit_count.saturating_add(1);
            next.push(insertion);
        }
    }

    fn expand_completion(&mut self, state: &State, next: &mut Vec<State>) {
        self.collect_children(&state.node);
        let child_count = self.children.len();
        for index in 0..child_count {
            let child = self.children[index];
            let char_count = child.char_count as usize;
            if char_count == 0 || state.word_len as usize + char_count > MAX_WORD_LENGTH {
                continue;
            }
            let completion_cost = if state.word_len == 0 {
                COST_FIRST_COMPLETION
            } else {
                COST_COMPLETION
            };
            let new_state =
                self.make_child_state(state, child, completion_cost, ERROR_COMPLETION, 0);
            next.push(new_state);
        }
    }

    /// True when every character of `node` equals the corresponding input point
    /// (case or accent differences aside).
    fn group_matches_input(&self, node: &Node, input_position: usize) -> bool {
        let char_count = node.char_count as usize;
        for offset in 0..char_count {
            let position = input_position + offset;
            if position >= self.input.len() {
                return false;
            }
            let primary = charutils::to_lower(self.input[position].primary);
            let candidate = charutils::to_lower(node.char_at(offset));
            let candidate_base = charutils::to_base_lower(node.char_at(offset));
            if candidate != primary && candidate_base != primary {
                return false;
            }
        }
        true
    }

    /// Builds the successor state for `child`, appending its characters and
    /// recording it when it completes a word for the consumed input.
    ///
    /// `consumed` is how many of the typed characters this child accounts for.
    /// Advancing the input index is what keeps the search on the typed path: a
    /// state that does not consume its characters matches the same key again and
    /// again and wanders through unrelated branches of the trie.
    fn make_child_state(
        &mut self,
        state: &State,
        child: Node,
        cost: f32,
        errors: u32,
        consumed: usize,
    ) -> State {
        let mut new_state = *state;
        new_state.node = child;
        new_state.spatial += cost;
        new_state.errors |= errors;
        if errors & ERROR_EDIT != 0 {
            new_state.edit_count = new_state.edit_count.saturating_add(1);
        }
        new_state.terminal_value = terminal_value_of(child, self.dict.is_v4());
        new_state.is_root = false;
        new_state.input_index = state.input_index.saturating_add(consumed as u16);
        self.push_word(&mut new_state, &child);
        let input_consumed = new_state.input_index as usize >= self.input.len();
        // A gesture has no typed characters to consume, so any terminal the path
        // reaches is a candidate. For typing, a word that ends before the typed
        // input does is only a prefix of it, not a suggestion.
        if child.is_terminal() && (self.options.is_gesture || input_consumed) {
            self.record_terminal(&new_state);
        }
        new_state
    }

    fn push_word(&self, state: &mut State, node: &Node) {
        let mut length = state.word_len as usize;
        for index in 0..node.char_count as usize {
            if length >= MAX_WORD_LENGTH {
                break;
            }
            state.word[length] = node.char_at(index);
            length += 1;
        }
        state.word_len = length as u8;
    }

    /// Gesture decoding: match the dictionary characters as a subsequence of
    /// the touch points, paying for every skipped point.
    fn expand_gesture(&mut self, state: &State, next: &mut Vec<State>) {
        if state.word_len as usize >= MAX_WORD_LENGTH {
            return;
        }

        self.collect_children(&state.node);
        let child_count = self.children.len();

        let cursor = state.input_index as usize;
        for index in 0..child_count {
            let child = self.children[index];
            if child.char_count != 1 {
                continue;
            }
            let wanted = charutils::to_lower(child.char_at(0));
            let mut found: Option<usize> = None;
            let limit = (cursor + GESTURE_MAX_POINT_LOOKAHEAD).min(self.input.len());
            let mut position = cursor;
            while position < limit {
                let point = &self.input[position];
                let proximity_type = self.proximity.proximity_type(
                    point.primary,
                    child.char_at(0),
                    point.x,
                    point.y,
                );
                if proximity_type != ProximityType::UnrelatedChar
                    || charutils::to_lower(point.primary) == wanted
                {
                    found = Some(position);
                    break;
                }
                position += 1;
            }
            let matched_position = match found {
                Some(value) => value,
                None => continue,
            };
            let point = &self.input[matched_position];
            let skipped = matched_position - cursor;
            let cost = self
                .proximity
                .distance_for_code_point(child.char_at(0), point.x, point.y)
                + (skipped as f32) * GESTURE_SKIP_COST;
            let errors = if skipped > 0 { ERROR_PROXIMITY } else { 0 };
            let new_state =
                self.make_child_state(state, child, cost, errors, matched_position + 1 - cursor);
            next.push(new_state);
        }
    }
}

fn terminal_value_of(node: Node, is_v4: bool) -> i32 {
    if is_v4 {
        node.terminal_id
    } else {
        node.probability
    }
}

fn add_result(
    results: &mut Vec<SuggestedWord>,
    code_points: &[i32],
    score: i32,
    kind: i32,
    index_to_partial_commit: i32,
    confidence: i32,
) {
    if code_points.is_empty() || code_points.len() > MAX_WORD_LENGTH {
        return;
    }
    // Reject duplicates, keeping the better score.
    for existing in results.iter_mut() {
        if existing.as_slice() == code_points {
            if score > existing.score {
                existing.score = score;
                existing.kind = kind;
                existing.auto_commit_first_word_confidence =
                    existing.auto_commit_first_word_confidence.max(confidence);
            }
            return;
        }
    }

    if results.len() >= MAX_RESULTS {
        // Replace the worst entry if this one is better.
        let mut worst_index = 0usize;
        let mut worst_score = i32::MAX;
        for (index, existing) in results.iter().enumerate() {
            if existing.score < worst_score {
                worst_score = existing.score;
                worst_index = index;
            }
        }
        if score <= worst_score {
            return;
        }
        results.remove(worst_index);
    }

    let mut word = SuggestedWord::empty();
    for (index, code_point) in code_points.iter().enumerate() {
        word.code_points[index] = *code_point;
    }
    word.length = code_points.len() as u8;
    word.score = score;
    word.kind = kind;
    word.index_to_partial_commit = index_to_partial_commit;
    word.auto_commit_first_word_confidence = confidence;
    results.push(word);
}

fn build_input_states(
    proximity: &ProximityInfo,
    points: &[(i32, i32, i32, i32)],
    input_code_points: &[i32],
) -> Vec<InputState> {
    let length = if input_code_points.is_empty() {
        points.len()
    } else {
        input_code_points.len()
    };
    let length = length.min(MAX_WORD_LENGTH);
    let mut states = Vec::with_capacity(length);
    for index in 0..length {
        let (x, y) = match points.get(index) {
            Some(point) => (point.0, point.1),
            None => (0, 0),
        };
        let mut proximity_chars = [0i32; MAX_PROXIMITY_CHARS_SIZE];
        let proximity_len = proximity.proximity_chars_at(x, y, &mut proximity_chars);
        // A non positive code point means no key was reported for this input
        // (gesture points between keys, or the `NOT_A_CODE` padding the Java
        // side leaves in the array). Fall back to the most likely key of the
        // cell instead of searching for a character that cannot exist.
        let primary = match input_code_points.get(index) {
            Some(value) if *value > 0 => *value,
            _ => {
                if proximity_len > 0 {
                    proximity_chars[0]
                } else {
                    0
                }
            }
        };
        states.push(InputState {
            primary,
            x,
            y,
            proximity: proximity_chars,
            proximity_len: proximity_len as u8,
        });
    }
    states
}

fn previous_terminal_ids(dict: &Dictionary, prev_words: &[Vec<i32>]) -> Vec<i32> {
    let mut ids = Vec::with_capacity(prev_words.len());
    for word in prev_words.iter().take(crate::MAX_PREV_WORD_COUNT_FOR_N_GRAM) {
        let (terminal_id, _node) = dict.find_word(word);
        if terminal_id == NOT_A_PROBABILITY {
            return Vec::new();
        }
        ids.push(terminal_id);
    }
    ids
}

/// Produces suggestions for the given input.
///
/// `points` are `(x, y, time, pointer_id)` quadruples; `input_code_points` are
/// the code points of the keys the points landed on.  `prev_words` are the
/// already committed words before the cursor, most recent first.
#[allow(clippy::too_many_arguments)]
pub fn get_suggestions(
    dict: &Dictionary,
    proximity: &ProximityInfo,
    points: &[(i32, i32, i32, i32)],
    input_code_points: &[i32],
    prev_words: &[Vec<i32>],
    options: &SuggestOptions,
    weight_of_lang_model_vs_spatial: &mut f32,
    results: &mut Vec<SuggestedWord>,
) {
    results.clear();
    if dict.is_corrupted() {
        return;
    }
    if *weight_of_lang_model_vs_spatial < 0.0f32 {
        *weight_of_lang_model_vs_spatial = 1.0f32;
    }

    let input = build_input_states(proximity, points, input_code_points);
    if input.is_empty() {
        get_predictions(dict, prev_words, results);
        return;
    }

    let prev_ids = previous_terminal_ids(dict, prev_words);
    let mut search = Search {
        dict,
        proximity,
        input: &input,
        options,
        prev_terminal_ids: &prev_ids,
        weight_of_lang_model_vs_spatial: *weight_of_lang_model_vs_spatial,
        results,
        children: Vec::with_capacity(64),
    };

    let mut active: Vec<State> = Vec::with_capacity(MAX_STATES);
    let mut next: Vec<State> = Vec::with_capacity(MAX_STATES);
    let root = State::root();

    if options.is_gesture {
        search.expand_gesture(&root, &mut next);
    } else {
        search.expand(&root, &mut next);
    }

    let max_steps = input.len() + MAX_EXTRA_COMPLETION_STEPS + 1;
    let mut step = 0usize;
    while !next.is_empty() && step < max_steps {
        step += 1;
        std::mem::swap(&mut active, &mut next);
        next.clear();
        for state in active.iter() {
            if options.is_gesture {
                search.expand_gesture(state, &mut next);
            } else {
                search.expand(state, &mut next);
            }
            // Every child of every state is now considered, so prune during the
            // step instead of letting the frontier grow to states x children.
            if next.len() >= MAX_FRONTIER {
                next.sort_by(|a, b| {
                    a.cost().partial_cmp(&b.cost()).unwrap_or(std::cmp::Ordering::Equal)
                });
                next.truncate(MAX_STATES);
            }
        }
        if next.len() > MAX_STATES {
            next.sort_by(|a, b| a.cost().partial_cmp(&b.cost()).unwrap_or(std::cmp::Ordering::Equal));
            next.truncate(MAX_STATES);
        }
        if search.results.len() >= MAX_TERMINALS {
            // Enough evidence; further expansion rarely improves ranking.
            break;
        }
    }

    // Always offer the personal dictionary entries that match the prefix.
    if let Some(user_dict) = dict.user.as_ref() {
        let mut prefix = [0i32; MAX_WORD_LENGTH];
        let prefix_len = input.len().min(MAX_WORD_LENGTH);
        for index in 0..prefix_len {
            prefix[index] = input[index].primary;
        }
        let prefix_slice = &prefix[..prefix_len];
        let mut collected: Vec<(SuggestedWord, i32)> = Vec::new();
        user_dict.for_each_prefixed(prefix_slice, |word, probability, flags| {
            let mut suggestion = SuggestedWord::empty();
            let length = word.len().min(MAX_WORD_LENGTH);
            for index in 0..length {
                suggestion.code_points[index] = word[index];
            }
            suggestion.length = length as u8;
            let kind = if flags & user::FLAG_POSSIBLY_OFFENSIVE != 0 {
                KIND_CORRECTION | KIND_FLAG_POSSIBLY_OFFENSIVE
            } else {
                KIND_CORRECTION
            };
            suggestion.kind = kind;
            suggestion.index_to_partial_commit = length as i32;
            collected.push((suggestion, probability));
        });
        for (mut suggestion, probability) in collected {
            let clamped = (probability as f32).max(1.0f32);
            let language_term = -(clamped / MAX_PROBABILITY_FOR_WEIGHTING).log10();
            let score = (TYPING_BASE_OUTPUT_SCORE
                - (language_term / (DISTANCE_WEIGHT_LANGUAGE + input.len() as f32 * 0.1f32)))
                * SUGGEST_INTERFACE_OUTPUT_SCALE;
            suggestion.score = score as i32;
            add_result(
                search.results,
                suggestion.as_slice(),
                suggestion.score,
                suggestion.kind,
                suggestion.index_to_partial_commit,
                0,
            );
        }
    }

    search
        .results
        .sort_by(|a, b| b.score.cmp(&a.score));
    if search.results.len() > MAX_RESULTS {
        search.results.truncate(MAX_RESULTS);
    }
}

/// Word predictions for an empty input field ("next word" suggestions).
///
/// Three sources, in this order:
///
/// 1. the n-gram table under the previous word, which is the only source that
///    knows "after *the*" is not the same as "after *I*".
/// 2. the cached most probable words, for the first word of a sentence.
/// 3. the trie itself, for a dictionary that carries no n-gram data at all.
///
/// Before this, predictions always came from 3, and 3 walks the trie in storage
/// order: the answer was "to/of/in" no matter what had been typed, and would
/// have been "aardvark" if the budget had run out earlier.
pub fn get_predictions(
    dict: &Dictionary,
    prev_words: &[Vec<i32>],
    results: &mut Vec<SuggestedWord>,
) {
    results.clear();
    if dict.is_corrupted() {
        return;
    }
    let prev_ids = previous_terminal_ids(dict, prev_words);

    let mut candidates: Vec<(SuggestedWord, i32)> = Vec::new();
    collect_ngram_predictions(dict, &prev_ids, &mut candidates);
    if candidates.is_empty() {
        collect_cached_predictions(dict, &mut candidates);
    }
    if candidates.is_empty() {
        // A dictionary without a language model: fall back to the trie walk.
        let mut budget = PREDICTION_NODE_BUDGET;
        collect_predictions(dict, &prev_ids, &mut budget, &mut candidates);
    }

    if let Some(user_dict) = dict.user.as_ref() {
        for record in user_dict.words() {
            if candidates.len() >= 256 {
                break;
            }
            let mut suggestion = SuggestedWord::empty();
            let length = record.word.len().min(MAX_WORD_LENGTH);
            for index in 0..length {
                suggestion.code_points[index] = record.word[index];
            }
            suggestion.length = length as u8;
            candidates.push((suggestion, record.probability));
        }
    }

    candidates.sort_by(|a, b| b.1.cmp(&a.1));
    for (mut suggestion, probability) in candidates.into_iter().take(MAX_RESULTS) {
        let clamped = (probability as f32).max(1.0f32);
        let language_term = -(clamped / MAX_PROBABILITY_FOR_WEIGHTING).log10();
        let score =
            (TYPING_BASE_OUTPUT_SCORE - language_term / DISTANCE_WEIGHT_LANGUAGE)
                * SUGGEST_INTERFACE_OUTPUT_SCALE;
        suggestion.score = score as i32;
        suggestion.kind = crate::KIND_PREDICTION;
        suggestion.index_to_partial_commit = suggestion.length as i32;
        results.push(suggestion);
    }
}

/// Fills `out` with the words the language model has seen after `prev_ids`.
fn collect_ngram_predictions(
    dict: &Dictionary,
    prev_ids: &[i32],
    out: &mut Vec<(SuggestedWord, i32)>,
) {
    if prev_ids.is_empty() {
        return;
    }
    let mut entries: Vec<(i32, i32, u8)> = Vec::new();
    dict.ngram_predictions(prev_ids, &mut entries);
    let mut buffer = [0i32; MAX_WORD_LENGTH];
    for (terminal_id, probability, _flags) in entries {
        if let Some(length) = dict.word_of_terminal_id(terminal_id, &mut buffer) {
            let mut suggestion = SuggestedWord::empty();
            suggestion.code_points[..length].copy_from_slice(&buffer[..length]);
            suggestion.length = length as u8;
            out.push((suggestion, probability));
        }
    }
}

/// Fills `out` with the cached most probable words of the dictionary.
fn collect_cached_predictions(dict: &Dictionary, out: &mut Vec<(SuggestedWord, i32)>) {
    let mut buffer = [0i32; MAX_WORD_LENGTH];
    for (terminal_id, probability) in dict.top_unigrams() {
        if out.len() >= MAX_RESULTS {
            break;
        }
        if let Some(length) = dict.word_of_terminal_id(*terminal_id, &mut buffer) {
            let mut suggestion = SuggestedWord::empty();
            suggestion.code_points[..length].copy_from_slice(&buffer[..length]);
            suggestion.length = length as u8;
            out.push((suggestion, *probability));
        }
    }
}

/// Last-resort walk of the trie, used only when the dictionary has no language
/// model data. Keeps its own node budget.
#[allow(clippy::needless_range_loop)]
fn collect_predictions(
    dict: &Dictionary,
    prev_ids: &[i32],
    budget: &mut usize,
    out: &mut Vec<(SuggestedWord, i32)>,
) {
    if *budget == 0 || out.len() >= 512 {
        return;
    }
    // This walk touches most of the trie, so ask for it up front instead of
    // taking a page fault every few nodes.
    dict.prefetch_trie();
    let root = Node {
        pos: 0,
        children_pos: 0,
        ..Node::empty()
    };

    // Breadth first, so every branch of the trie gets a share of the budget.
    // Each entry carries its own prefix in a fixed-size buffer: a shared one
    // gave every queued node the prefix of whatever was walked last ("éypt"
    // for "Egypt"), and a `Vec` per node allocated on every keystroke.
    let mut queue: std::collections::VecDeque<(Node, Prefix)> =
        std::collections::VecDeque::with_capacity(PREDICTION_QUEUE_LIMIT);
    dict.for_each_child(&root, |child| {
        if child.char_count > 0 {
            let mut prefix = Prefix::empty();
            prefix.push_node(child);
            queue.push_back((*child, prefix));
        }
        true
    });

    while let Some((node, prefix)) = queue.pop_front() {
        if *budget == 0 || out.len() >= 512 {
            return;
        }
        *budget -= 1;

        if node.is_terminal() {
            let terminal = terminal_value_of(node, dict.is_v4());
            let attributes = dict.word_attributes(prev_ids, terminal);
            if attributes.is_valid() && !attributes.is_not_a_word && !attributes.is_blacklisted {
                out.push((prefix.to_suggestion(), attributes.probability));
            }
        }

        dict.for_each_child(&node, |child| {
            if queue.len() >= PREDICTION_QUEUE_LIMIT {
                return false;
            }
            if child.char_count > 0 && prefix.len as usize + child.char_count as usize <= MAX_WORD_LENGTH
            {
                let mut child_prefix = prefix;
                child_prefix.push_node(child);
                queue.push_back((*child, child_prefix));
            }
            true
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_option_array() {
        let options = SuggestOptions::from_array(&[1, 0, 1, 0, 1500]);
        assert!(options.is_gesture);
        assert!(!options.use_full_edit_distance);
        assert!(options.block_offensive_words);
        assert!((options.weight_for_locale - 1.5f32).abs() < 0.0001f32);
    }

    #[test]
    fn results_keep_the_best_score() {
        let mut results: Vec<SuggestedWord> = Vec::new();
        add_result(&mut results, &[0x61], 100, KIND_TYPED, 1, 0);
        add_result(&mut results, &[0x61], 200, KIND_TYPED, 1, 0);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].score, 200);
    }

    #[test]
    fn results_are_capped() {
        let mut results: Vec<SuggestedWord> = Vec::new();
        for index in 0..(MAX_RESULTS + 20) {
            add_result(&mut results, &[0x61 + index as i32], index as i32, 0, 1, 0);
        }
        assert_eq!(results.len(), MAX_RESULTS);
    }

    // --- end to end behaviour on a real (in memory) trie --------------------

    use crate::dict::{from_v2_trie_for_test, test_trie};
    use crate::proximity::ProximityInfo;

    const KEY_WIDTH: i32 = 100;

    fn letters() -> Vec<i32> {
        ('a'..='z').map(|c| c as i32).collect()
    }

    /// A single row of 26 keys, one hundred units wide.
    fn row_proximity() -> ProximityInfo {
        let count = letters().len();
        let key_x: Vec<i32> = (0..count as i32).map(|index| index * KEY_WIDTH).collect();
        let key_y = vec![0; count];
        let key_widths = vec![KEY_WIDTH; count];
        let key_heights = vec![100; count];
        let sweet_x: Vec<f32> = (0..count as i32)
            .map(|index| index as f32 * KEY_WIDTH as f32 + KEY_WIDTH as f32 / 2.0)
            .collect();
        let sweet_y = vec![50.0f32; count];
        let sweet_radius = vec![50.0f32; count];
        ProximityInfo::new(
            count as i32 * KEY_WIDTH,
            100,
            count as i32,
            1,
            KEY_WIDTH,
            100,
            Vec::new(),
            count,
            key_x,
            key_y,
            key_widths,
            key_heights,
            letters(),
            sweet_x,
            sweet_y,
            sweet_radius,
        )
    }

    fn taps(typed: &str) -> Vec<(i32, i32, i32, i32)> {
        typed
            .chars()
            .enumerate()
            .map(|(index, ch)| {
                let column = (ch as i32) - ('a' as i32);
                (
                    column * KEY_WIDTH + KEY_WIDTH / 2,
                    50,
                    index as i32 * 50,
                    0,
                )
            })
            .collect()
    }

    fn dictionary() -> (crate::dict::Dictionary, Vec<String>) {
        let words: Vec<(&str, u8)> = vec![
            ("hello", 200),
            ("help", 190),
            ("helm", 150),
            ("helmet", 120),
            ("hero", 100),
            ("world", 90),
            ("held", 80),
        ];
        let trie = test_trie::build(&words);
        let names = words.iter().map(|(word, _)| word.to_string()).collect();
        (from_v2_trie_for_test(trie), names)
    }

    fn run(dict: &crate::dict::Dictionary, typed: &str) -> Vec<String> {
        let proximity = row_proximity();
        let code_points: Vec<i32> = typed.chars().map(|c| c as i32).collect();
        let mut weight = 1.0f32;
        let mut results = Vec::new();
        get_suggestions(
            dict,
            &proximity,
            &taps(typed),
            &code_points,
            &[],
            &SuggestOptions::defaults(),
            &mut weight,
            &mut results,
        );
        results
            .iter()
            .map(|word| {
                word.code_points[..word.length as usize]
                    .iter()
                    .map(|cp| char::from_u32(*cp as u32).unwrap_or('?'))
                    .collect::<String>()
            })
            .collect()
    }

    #[test]
    fn every_suggestion_is_a_dictionary_word() {
        let (dict, names) = dictionary();
        for typed in ["h", "he", "hel", "hell", "hello", "hellp", "helo", "hlep"] {
            for suggestion in run(&dict, typed) {
                assert!(
                    names.contains(&suggestion),
                    "typed {typed:?} produced {suggestion:?}, which is not in the dictionary"
                );
            }
        }
    }

    #[test]
    fn suggestions_stay_on_the_typed_prefix() {
        let (dict, _names) = dictionary();
        let results = run(&dict, "hel");
        assert!(!results.is_empty(), "no suggestions for \"hel\"");
        let top = &results[0];
        assert!(
            top.starts_with("hel"),
            "first suggestion for \"hel\" was {top:?}"
        );
        assert!(
            results[..3.min(results.len())]
                .iter()
                .all(|word| word.starts_with("hel")),
            "the best suggestions for \"hel\" were not completions: {results:?}"
        );
        for word in &results {
            // One edit away is a legitimate correction, "world" is not.
            assert!(
                word.starts_with('h'),
                "\"hel\" suggested {word:?}, which does not even start with the typed letter"
            );
        }
        assert!(results.contains(&"hello".to_string()));
    }

    #[test]
    fn typos_are_corrected_to_the_expected_word() {
        let (dict, _names) = dictionary();
        let results = run(&dict, "hellp");
        assert!(
            results.contains(&"hello".to_string()),
            "\"hellp\" did not correct to hello: {results:?}"
        );
    }

    #[test]
    fn unrelated_input_does_not_produce_words() {
        let (dict, _names) = dictionary();
        assert!(run(&dict, "qz").is_empty(), "\"qz\" produced suggestions");
    }
}

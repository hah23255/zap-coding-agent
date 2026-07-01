/// Ratatui rendering split into focused submodules.
mod header;
mod layout;
mod messages;
mod overlays;
mod provider_picker;
mod diff;
mod dialogs;
mod context_viewer;
mod init_wizard;

// Re-export the public surface that callers outside this module depend on.
pub use diff::open_diff_viewer;
pub use messages::{
    render_all_lines, message_to_lines, diff_block_lines, role_line,
    text_to_lines, code_block_lines, tool_call_lines,
    thinking_streaming_lines, thinking_collapsed_line,
};

use ratatui::{prelude::*, widgets::*};
use super::app::App;

pub const SPINNER_FRAMES: &[&str] = &[
    "⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏",
];

/// Words that rotate while the LLM is generating — changes roughly every 3s at 16ms tick.
const THINKING_WORDS: &[&str] = &[
    // Cognitive core
    "Thinking",        "Analyzing",       "Reasoning",       "Reflecting",
    "Considering",     "Contemplating",   "Pondering",       "Deliberating",
    "Cogitating",      "Musing",          "Speculating",     "Theorizing",
    "Inferring",       "Deducing",        "Synthesizing",    "Integrating",
    "Processing",      "Computing",       "Calculating",     "Estimating",
    // Creative / generative
    "Planning",        "Drafting",        "Designing",       "Architecting",
    "Brainstorming",   "Ideating",        "Conceptualizing", "Envisioning",
    "Formulating",     "Composing",       "Constructing",    "Crafting",
    "Generating",      "Developing",      "Imagining",       "Prototyping",
    "Sketching",       "Outlining",       "Scaffolding",     "Blueprinting",
    // Analytical
    "Evaluating",      "Reviewing",       "Inspecting",      "Auditing",
    "Examining",       "Scrutinizing",    "Investigating",   "Researching",
    "Studying",        "Parsing",         "Decoding",        "Interpreting",
    "Comprehending",   "Absorbing",       "Assessing",       "Comparing",
    "Contrasting",     "Distinguishing",  "Benchmarking",    "Profiling",
    // Problem-solving
    "Solving",         "Debugging",       "Troubleshooting", "Diagnosing",
    "Probing",         "Testing",         "Verifying",       "Validating",
    "Correcting",      "Patching",        "Refining",        "Improving",
    "Optimizing",      "Enhancing",       "Tuning",          "Adjusting",
    "Calibrating",     "Fixing",          "Resolving",       "Untangling",
    // Code-specific
    "Refactoring",     "Rewriting",       "Abstracting",     "Mapping",
    "Tracing",         "Traversing",      "Navigating",      "Compiling",
    "Linting",         "Modularizing",    "Encapsulating",   "Decoupling",
    "Wiring",          "Bootstrapping",   "Instrumenting",   "Annotating",
    // Organizing
    "Documenting",     "Organizing",      "Structuring",     "Arranging",
    "Sequencing",      "Categorizing",    "Classifying",     "Sorting",
    "Filtering",       "Locating",        "Identifying",     "Recognizing",
    "Correlating",     "Connecting",      "Associating",     "Contextualizing",
    "Framing",         "Scoping",         "Prioritizing",    "Grouping",
    // Gathering / retrieval
    "Clustering",      "Gathering",       "Collecting",      "Aggregating",
    "Summarizing",     "Distilling",      "Extracting",      "Deriving",
    "Projecting",      "Approximating",   "Retrieving",      "Querying",
    "Fetching",        "Loading",         "Indexing",        "Searching",
    // Exploratory
    "Exploring",       "Discovering",     "Uncovering",      "Revealing",
    "Illuminating",    "Elucidating",     "Deciphering",     "Unraveling",
    "Dissecting",      "Deconstructing",  "Reconstructing",  "Reframing",
    "Rethinking",      "Reimagining",     "Revisiting",      "Deep-diving",
    // Verification / hardening
    "Cross-checking",  "Fact-checking",   "Sanity-checking", "Stress-testing",
    "Hardening",       "Streamlining",    "Consolidating",   "Normalizing",
    "Standardizing",   "Harmonizing",     "Aligning",        "Balancing",
    "Weighing",        "Confirming",      "Polishing",       "Finalizing",
    // Clarifying / finishing
    "Simplifying",     "Clarifying",      "Disambiguating",  "Reconciling",
    "Merging",         "Combining",       "Explaining",      "Modeling",
    "Simulating",      "Forecasting",     "Measuring",       "Quantifying",
    "Experimenting",   "Hypothesizing",   "Homing in",       "Focusing",
    // Meta / flow
    "Concentrating",   "Drilling down",   "Backtracking",    "Unpacking",
    "Decomposing",     "Iterating",       "Converging",      "Coalescing",
    "Scanning",        "Vetting",         "Extrapolating",   "Interpolating",
];

fn tool_verb(name: &str) -> &'static str {
    match name {
        "read_file"        => "Reading",
        "write_file"       => "Writing",
        "edit_file"        => "Editing",
        "batch_edit"       => "Editing",
        "undo_edit"        => "Undoing",
        "shell"            => "Running",
        "search_code"      => "Searching",
        "find_definition"  => "Looking up",
        "code_map"         => "Mapping",
        "list_directory"   => "Browsing",
        "web_fetch"        => "Fetching",
        "web_search"       => "Searching web",
        "spawn_agent"      => "Spawning agent",
        "read_memory"      => "Recalling",
        "write_memory"     => "Remembering",
        "index"            => "Indexing",
        _                  => "Running",
    }
}

/// Width of the right sidebar (includes the left border character).
pub const SIDEBAR_W: u16 = 22;

/// Max rows the command picker occupies (excluding its own border).
const PICKER_MAX_ROWS: usize = 8;

/// Hard-wrap `text` into display rows at exactly `content_w` columns, and
/// locate the cursor's (row, col) within that same layout. Row 0 starts at
/// column `prefix_chars` (reserved for the "❯ " prompt); every other row
/// starts at column 0.
///
/// Building the rows and the cursor position in one pass is what keeps them
/// from disagreeing. The input box used to compute the cursor's screen
/// position with this hard-wrap math while handing the *text* to ratatui's
/// `Paragraph::wrap()`, which word-wraps — so whenever a wrap landed mid-word
/// the two disagreed on where the line broke, and the blinking cursor
/// visibly drifted away from the character you'd just typed. Now both the
/// rendered rows and the cursor position come from this single function.
pub(crate) fn wrap_input(
    text: &str,
    cursor_char: usize,
    prefix_chars: usize,
    content_w: usize,
) -> (Vec<String>, usize, usize) {
    if content_w == 0 {
        return (vec![String::new()], 0, 0);
    }
    let mut rows: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut col = prefix_chars;
    let mut cursor_row = 0usize;
    let mut cursor_col = prefix_chars;
    let mut cursor_set = false;

    for (i, ch) in text.chars().enumerate() {
        if i == cursor_char {
            cursor_row = rows.len();
            cursor_col = col;
            cursor_set = true;
        }
        if ch == '\n' {
            rows.push(std::mem::take(&mut current));
            col = 0;
            continue;
        }
        current.push(ch);
        col += 1;
        if col >= content_w {
            rows.push(std::mem::take(&mut current));
            col = 0;
        }
    }
    if !cursor_set {
        cursor_row = rows.len();
        cursor_col = col;
    }
    rows.push(current);
    (rows, cursor_row, cursor_col)
}

fn input_height(app: &App, available_width: u16) -> Constraint {
    let prefix_len = 2usize;
    let border_w  = 2usize;
    let content_w = (available_width as usize).saturating_sub(prefix_len + border_w).max(1);
    let (rows, _, _) = wrap_input(&app.input, app.cursor, prefix_len, content_w);
    let lines = rows.len().clamp(1, 3);
    Constraint::Length(lines as u16 + 2)
}

pub fn draw(frame: &mut Frame, app: &App) {
    let size = frame.area();

    let outer = Layout::vertical([
        Constraint::Length(7),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .split(size);

    header::draw_header(frame, app, outer[0]);
    layout::draw_status(frame, app, outer[2]);

    let banner_h = if app.topic_shift_confirm.is_some() { 3u16 } else { 0u16 };

    let use_sidebar = size.width > SIDEBAR_W + 24;
    if use_sidebar {
        let body = Layout::horizontal([
            Constraint::Min(24),
            Constraint::Length(SIDEBAR_W),
        ])
        .split(outer[1]);

        let input_h = input_height(app, body[0].width);
        let left = Layout::vertical([
            Constraint::Min(1),
            Constraint::Length(banner_h),
            input_h,
            Constraint::Length(3),
        ])
        .split(body[0]);

        layout::draw_messages(frame, app, left[0]);
        layout::draw_picker_overlay(frame, app, left[0]);
        if app.topic_shift_confirm.is_some() {
            layout::draw_topic_shift_banner(frame, app, left[1]);
        }
        let cursor_pos = layout::draw_input(frame, app, left[2]);
        layout::draw_dir_panel(frame, app, left[3]);
        layout::draw_sidebar(frame, app, body[1]);
        layout::maybe_set_cursor(frame, app, cursor_pos);
    } else {
        // Clear the area where the sidebar would have been to avoid ghost characters.
        let sidebar_ghost = Rect {
            x: outer[1].x + outer[1].width.saturating_sub(SIDEBAR_W),
            y: outer[1].y,
            width: SIDEBAR_W,
            height: outer[1].height,
        };
        frame.render_widget(Clear, sidebar_ghost);

        let left = Layout::vertical([
            Constraint::Min(1),
            Constraint::Length(banner_h),
            input_height(app, outer[1].width),
            Constraint::Length(6),
        ])
        .split(outer[1]);

        layout::draw_messages(frame, app, left[0]);
        layout::draw_picker_overlay(frame, app, left[0]);
        if app.topic_shift_confirm.is_some() {
            layout::draw_topic_shift_banner(frame, app, left[1]);
        }
        let cursor_pos = layout::draw_input(frame, app, left[2]);
        layout::draw_dir_panel(frame, app, left[3]);
        layout::maybe_set_cursor(frame, app, cursor_pos);
    }

    if app.file_browser.is_some() {
        overlays::draw_file_browser(frame, app, size);
    }

    if app.mode_picker.is_some() {
        overlays::draw_mode_picker(frame, app, size);
        return;
    }

    if app.domain_picker.is_some() {
        overlays::draw_domain_picker(frame, app, size);
    }

    if app.session_picker.is_some() {
        overlays::draw_session_picker(frame, app, size);
    }

    if app.provider_picker.is_some() {
        provider_picker::draw_provider_picker(frame, app, size);
    }

    if app.init_wizard.is_some() {
        init_wizard::draw_init_wizard(frame, app, size);
    }

    if app.diff_viewer.is_some() {
        diff::draw_diff_viewer(frame, app, size);
        return;
    }

    if app.command_popup.is_some() {
        dialogs::draw_command_popup(frame, app, size);
    }

    if app.permission_popup.is_some() {
        dialogs::draw_permission_popup(frame, app, size);
    }

    if app.btw_mode {
        dialogs::draw_btw_input(frame, app, size);
    }

    if app.gemini_auth_prompt {
        overlays::draw_gemini_auth_prompt(frame, app, size);
    }

    if app.api_key_input.is_some() {
        overlays::draw_api_key_input(frame, app, size);
    }

    if app.context_viewer.is_some() {
        context_viewer::draw_context_viewer(frame, app, size);
    }

    if app.file_picker.is_some() {
        overlays::draw_file_picker(frame, app, size);
    }
}

#[cfg(test)]
mod wrap_input_tests {
    use super::wrap_input;

    #[test]
    fn short_text_single_row() {
        let (rows, row, col) = wrap_input("hi", 2, 2, 20);
        assert_eq!(rows, vec!["hi".to_string()]);
        assert_eq!((row, col), (0, 4)); // prefix(2) + "hi"
    }

    #[test]
    fn wraps_mid_word_not_at_space() {
        // content_w=10, prefix=2 → row 0 holds 8 chars before wrapping,
        // even though that lands in the middle of "abcdefghij".
        let (rows, _, _) = wrap_input("abcdefghij", 10, 2, 10);
        assert_eq!(rows, vec!["abcdefgh".to_string(), "ij".to_string()]);
    }

    #[test]
    fn cursor_after_wrap_point_lands_on_row_1() {
        let (rows, row, col) = wrap_input("abcdefghij", 9, 2, 10);
        assert_eq!(rows, vec!["abcdefgh".to_string(), "ij".to_string()]);
        assert_eq!((row, col), (1, 1)); // 'j' is the 2nd char of row 1, col index 1
    }

    #[test]
    fn explicit_newline_resets_column_to_zero() {
        let (rows, row, col) = wrap_input("ab\ncd", 5, 2, 10);
        assert_eq!(rows, vec!["ab".to_string(), "cd".to_string()]);
        assert_eq!((row, col), (1, 2)); // cursor at end of "cd"
    }

    #[test]
    fn empty_text_is_one_empty_row() {
        let (rows, row, col) = wrap_input("", 0, 2, 10);
        assert_eq!(rows, vec![String::new()]);
        assert_eq!((row, col), (0, 2));
    }

    #[test]
    fn zero_width_does_not_panic() {
        let (rows, row, col) = wrap_input("anything", 3, 2, 0);
        assert_eq!(rows, vec![String::new()]);
        assert_eq!((row, col), (0, 0));
    }
}

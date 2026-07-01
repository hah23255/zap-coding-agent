//! Keyword-heuristic task type classification. No LLM call, no allocations beyond the lowercase copy.

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum TaskType {
    Coding,
    Review,
    Explain,
    Search,
    Default,
}

impl TaskType {
    pub fn as_str(self) -> &'static str {
        match self {
            TaskType::Coding  => "coding",
            TaskType::Review  => "review",
            TaskType::Explain => "explain",
            TaskType::Search  => "search",
            TaskType::Default => "default",
        }
    }
}

/// Classify the user's input into a task type using keyword heuristics.
pub fn classify(input: &str) -> TaskType {
    let lower = input.to_lowercase();
    if is_review(&lower)  { return TaskType::Review; }
    if is_explain(&lower) { return TaskType::Explain; }
    if is_search(&lower)  { return TaskType::Search; }
    if is_coding(&lower)  { return TaskType::Coding; }
    TaskType::Default
}

fn is_coding(s: &str) -> bool {
    ["implement", "write a function", "write the", "add a ", "fix the bug",
     "refactor", "create a ", "build ", "write code", "make it work",
     "add feature", "add support", "add test", "add tests"]
        .iter().any(|k| s.contains(k))
}

fn is_review(s: &str) -> bool {
    ["review", "check my code", "look at this", "what do you think of",
     "is this correct", "any issues", "code review", "feedback on",
     "does this look", "does this work"]
        .iter().any(|k| s.contains(k))
}

fn is_explain(s: &str) -> bool {
    ["explain", "what is", "what does", "how does", "why does", "what's the",
     "describe", "summarize", "tell me about", "how do i", "what are"]
        .iter().any(|k| s.contains(k))
}

fn is_search(s: &str) -> bool {
    ["find", "search", "where is", "where does", "grep", "look for",
     "which file", "which function", "where can i"]
        .iter().any(|k| s.contains(k))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coding_inputs_classified() {
        assert_eq!(classify("implement the login function"), TaskType::Coding);
        assert_eq!(classify("write a function to sort users"), TaskType::Coding);
        assert_eq!(classify("fix the bug in auth.rs"), TaskType::Coding);
        assert_eq!(classify("add tests for the parser"), TaskType::Coding);
    }

    #[test]
    fn review_inputs_classified() {
        assert_eq!(classify("please review my pull request"), TaskType::Review);
        assert_eq!(classify("can you code review this?"), TaskType::Review);
        assert_eq!(classify("what do you think of this approach"), TaskType::Review);
    }

    #[test]
    fn explain_inputs_classified() {
        assert_eq!(classify("explain how the context manager works"), TaskType::Explain);
        assert_eq!(classify("what is the difference between X and Y"), TaskType::Explain);
        assert_eq!(classify("how does session resumption work"), TaskType::Explain);
    }

    #[test]
    fn search_inputs_classified() {
        assert_eq!(classify("find where save_context is called"), TaskType::Search);
        assert_eq!(classify("which file handles MCP connections"), TaskType::Search);
    }

    #[test]
    fn default_for_ambiguous() {
        assert_eq!(classify("hi"), TaskType::Default);
        assert_eq!(classify("deploy please"), TaskType::Default);
        assert_eq!(classify("show me the logs"), TaskType::Default);
    }

    #[test]
    fn review_takes_priority_over_explain() {
        // "review" should beat "what does" ordering
        assert_eq!(classify("review what does this function do"), TaskType::Review);
    }
}

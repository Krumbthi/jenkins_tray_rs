//! Error presentation helpers.

/// The deepest cause of an error chain, as a single line.
///
/// `anyhow`'s alternate formatting (`{e:#}`) appends every source, but
/// reqwest/hyper already embed their own causes in their `Display`, so a
/// plain DNS failure would be printed three times in one log line (this is
/// how the original error reports read:
/// `...: error trying to connect: dns error: ...: error trying to connect:
/// dns error: ...: dns error: ...: failed to lookup ...`).
pub fn root_cause(error: &anyhow::Error) -> String {
    error
        .chain()
        .last()
        .map(|cause| cause.to_string())
        .unwrap_or_else(|| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_cause_is_only_the_deepest_source() {
        let inner = std::io::Error::new(std::io::ErrorKind::NotFound, "the real cause");
        let error: anyhow::Error = anyhow::Error::new(inner).context("request failed");

        assert_eq!(root_cause(&error), "the real cause");
        assert!(!root_cause(&error).contains("request failed"));
    }
}

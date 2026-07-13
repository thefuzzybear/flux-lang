//! Hypotheses document parsing and updating (`hypotheses.md`).

use super::NucleusError;

/// The full hypotheses document: active hypotheses and killed hypotheses.
#[derive(Debug, Clone, PartialEq)]
pub struct HypothesisDocument {
    pub active: Vec<Hypothesis>,
    pub killed: Vec<KilledHypothesis>,
}

/// An active hypothesis with a claim and predictions.
#[derive(Debug, Clone, PartialEq)]
pub struct Hypothesis {
    pub id: String,
    pub claim: String,
    pub predictions: Vec<Prediction>,
}

/// A single prediction within a hypothesis.
#[derive(Debug, Clone, PartialEq)]
pub struct Prediction {
    pub number: u32,
    pub text: String,
    pub trial: String,
    pub result: String,
    pub verdict: VerdictStatus,
}

/// The verdict status for a prediction.
#[derive(Debug, Clone, PartialEq)]
pub enum VerdictStatus {
    Pending,
    Survived,
    Killed(String),
}

/// A hypothesis that has been killed (all predictions failed).
#[derive(Debug, Clone, PartialEq)]
pub struct KilledHypothesis {
    pub id: String,
    pub claim: String,
    pub reason: String,
}

/// Parse a hypotheses.md string into a HypothesisDocument.
pub fn parse_hypotheses(content: &str) -> Result<HypothesisDocument, NucleusError> {
    // Split into active and killed sections
    let (active_section, killed_section) = split_sections(content)?;

    let active = parse_active_section(&active_section)?;
    let killed = parse_killed_section(&killed_section)?;

    Ok(HypothesisDocument { active, killed })
}

/// Serialize a HypothesisDocument back to markdown string.
pub fn serialize_hypotheses(doc: &HypothesisDocument) -> String {
    let mut output = String::new();
    output.push_str("# Hypotheses\n\n## Active\n\n");

    if doc.active.is_empty() {
        output.push_str("(None yet — run discovery cells to generate hypotheses)\n");
    } else {
        for hyp in &doc.active {
            output.push_str(&format!("### {}: {}\n\n", hyp.id, hyp.claim));
            output.push_str("| # | Prediction | Trial | Result | Verdict |\n");
            output.push_str("|---|-----------|-------|--------|----------|\n");
            for pred in &hyp.predictions {
                let verdict_str = serialize_verdict(&pred.verdict);
                output.push_str(&format!(
                    "| {} | {} | {} | {} | {} |\n",
                    pred.number, pred.text, pred.trial, pred.result, verdict_str
                ));
            }
            output.push('\n');
        }
    }

    output.push_str("## Killed\n\n");

    if doc.killed.is_empty() {
        output.push_str("(Empty — no hypotheses tested yet)\n");
    } else {
        for killed in &doc.killed {
            output.push_str(&format!("### {}: {}\n", killed.id, killed.claim));
            output.push_str(&format!("Reason: {}\n\n", killed.reason));
        }
    }

    output
}

/// Update a specific prediction's verdict by matching the trial name.
pub fn update_prediction_verdict(
    doc: &mut HypothesisDocument,
    trial_name: &str,
    verdict: VerdictStatus,
) -> Result<(), NucleusError> {
    for hyp in &mut doc.active {
        for pred in &mut hyp.predictions {
            if pred.trial == trial_name {
                pred.verdict = verdict;
                return Ok(());
            }
        }
    }
    Err(NucleusError::HypothesesParseError(format!(
        "trial '{}' not found in any active hypothesis prediction",
        trial_name
    )))
}

/// If all predictions of a hypothesis are Killed, move it to the killed section.
pub fn check_and_move_killed(doc: &mut HypothesisDocument) {
    let mut i = 0;
    while i < doc.active.len() {
        let hyp = &doc.active[i];
        if !hyp.predictions.is_empty()
            && hyp
                .predictions
                .iter()
                .all(|p| matches!(&p.verdict, VerdictStatus::Killed(_)))
        {
            let removed = doc.active.remove(i);
            // Use the reason from the last killed prediction
            let reason = removed
                .predictions
                .iter()
                .filter_map(|p| match &p.verdict {
                    VerdictStatus::Killed(r) => Some(r.clone()),
                    _ => None,
                })
                .last()
                .unwrap_or_default();
            doc.killed.push(KilledHypothesis {
                id: removed.id,
                claim: removed.claim,
                reason,
            });
        } else {
            i += 1;
        }
    }
}

/// Returns (total, tested, survived) for all active hypotheses' predictions.
pub fn prediction_stats(doc: &HypothesisDocument) -> (u32, u32, u32) {
    let mut total = 0u32;
    let mut tested = 0u32;
    let mut survived = 0u32;

    for hyp in &doc.active {
        for pred in &hyp.predictions {
            total += 1;
            match &pred.verdict {
                VerdictStatus::Pending => {}
                VerdictStatus::Survived => {
                    tested += 1;
                    survived += 1;
                }
                VerdictStatus::Killed(_) => {
                    tested += 1;
                }
            }
        }
    }

    (total, tested, survived)
}

// --- Internal helpers ---

fn split_sections(content: &str) -> Result<(String, String), NucleusError> {
    // Find "## Active" and "## Killed" headers
    let active_start = content.find("## Active").ok_or_else(|| {
        NucleusError::HypothesesParseError("missing '## Active' section".to_string())
    })?;

    let killed_start = content.find("## Killed").ok_or_else(|| {
        NucleusError::HypothesesParseError("missing '## Killed' section".to_string())
    })?;

    // Active section is between "## Active" header and "## Killed" header
    let active_header_end = active_start + "## Active".len();
    let active_section = content[active_header_end..killed_start].to_string();

    // Killed section is after "## Killed" header
    let killed_header_end = killed_start + "## Killed".len();
    let killed_section = content[killed_header_end..].to_string();

    Ok((active_section, killed_section))
}

fn parse_active_section(section: &str) -> Result<Vec<Hypothesis>, NucleusError> {
    let mut hypotheses = Vec::new();

    // Split on "### H" to find hypothesis blocks
    let blocks: Vec<&str> = section.split("### ").collect();

    for block in blocks.iter().skip(1) {
        // Each block starts with "H<n>: <claim>\n..."
        let hyp = parse_hypothesis_block(block)?;
        hypotheses.push(hyp);
    }

    Ok(hypotheses)
}

fn parse_hypothesis_block(block: &str) -> Result<Hypothesis, NucleusError> {
    let mut lines = block.lines();

    // First line: "H<n>: <claim>"
    let header = lines.next().ok_or_else(|| {
        NucleusError::HypothesesParseError("empty hypothesis block".to_string())
    })?;

    let (id, claim) = parse_hypothesis_header(header)?;

    // Parse prediction table
    let predictions = parse_prediction_table(&mut lines)?;

    Ok(Hypothesis {
        id,
        claim,
        predictions,
    })
}

fn parse_hypothesis_header(header: &str) -> Result<(String, String), NucleusError> {
    // Format: "H<n>: <claim text>"
    let colon_pos = header.find(": ").ok_or_else(|| {
        NucleusError::HypothesesParseError(format!(
            "hypothesis header missing ': ' separator: '{}'",
            header
        ))
    })?;

    let id = header[..colon_pos].trim().to_string();
    let claim = header[colon_pos + 2..].trim().to_string();

    Ok((id, claim))
}

fn parse_prediction_table(
    lines: &mut std::str::Lines<'_>,
) -> Result<Vec<Prediction>, NucleusError> {
    let mut predictions = Vec::new();
    let mut in_table = false;
    let mut header_seen = false;

    for line in lines {
        let trimmed = line.trim();

        if trimmed.is_empty() {
            if in_table {
                break; // End of table
            }
            continue;
        }

        // Detect table header row
        if trimmed.starts_with("| #") || trimmed.starts_with("| # |") {
            in_table = true;
            header_seen = true;
            continue;
        }

        // Skip separator row
        if header_seen && trimmed.starts_with("|---") || trimmed.starts_with("|-") {
            continue;
        }

        // Parse data row
        if in_table && trimmed.starts_with('|') {
            let pred = parse_prediction_row(trimmed)?;
            predictions.push(pred);
        }
    }

    Ok(predictions)
}

fn parse_prediction_row(row: &str) -> Result<Prediction, NucleusError> {
    // Split on '|' and trim each cell. The leading/trailing '|' produce empty
    // first and last elements which we skip, but we keep interior empty cells.
    let raw_cells: Vec<&str> = row.split('|').collect();

    // Strip the first and last empty entries from leading/trailing '|'
    let cells: Vec<&str> = if raw_cells.len() >= 2 {
        raw_cells[1..raw_cells.len() - 1]
            .iter()
            .map(|s| s.trim())
            .collect()
    } else {
        raw_cells.iter().map(|s| s.trim()).collect()
    };

    if cells.len() < 5 {
        return Err(NucleusError::HypothesesParseError(format!(
            "prediction row has fewer than 5 columns: '{}'",
            row
        )));
    }

    let number: u32 = cells[0].parse().map_err(|_| {
        NucleusError::HypothesesParseError(format!(
            "invalid prediction number: '{}'",
            cells[0]
        ))
    })?;

    let text = cells[1].to_string();
    let trial = cells[2].to_string();
    let result = cells[3].to_string();
    let verdict = parse_verdict_status(cells[4])?;

    Ok(Prediction {
        number,
        text,
        trial,
        result,
        verdict,
    })
}

fn parse_verdict_status(s: &str) -> Result<VerdictStatus, NucleusError> {
    let trimmed = s.trim().to_lowercase();
    if trimmed == "pending" || trimmed.is_empty() {
        Ok(VerdictStatus::Pending)
    } else if trimmed == "survived" {
        Ok(VerdictStatus::Survived)
    } else if let Some(reason) = trimmed.strip_prefix("killed:") {
        Ok(VerdictStatus::Killed(reason.trim().to_string()))
    } else {
        Err(NucleusError::HypothesesParseError(format!(
            "invalid verdict status: '{}'",
            s
        )))
    }
}

fn serialize_verdict(verdict: &VerdictStatus) -> String {
    match verdict {
        VerdictStatus::Pending => "pending".to_string(),
        VerdictStatus::Survived => "survived".to_string(),
        VerdictStatus::Killed(reason) => format!("killed: {}", reason),
    }
}

fn parse_killed_section(section: &str) -> Result<Vec<KilledHypothesis>, NucleusError> {
    let mut killed = Vec::new();

    let blocks: Vec<&str> = section.split("### ").collect();

    for block in blocks.iter().skip(1) {
        let killed_hyp = parse_killed_block(block)?;
        killed.push(killed_hyp);
    }

    Ok(killed)
}

fn parse_killed_block(block: &str) -> Result<KilledHypothesis, NucleusError> {
    let mut lines = block.lines();

    // First line: "H<n>: <claim>"
    let header = lines.next().ok_or_else(|| {
        NucleusError::HypothesesParseError("empty killed hypothesis block".to_string())
    })?;

    let (id, claim) = parse_hypothesis_header(header)?;

    // Next non-empty line should be "Reason: <reason>"
    let mut reason = String::new();
    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(r) = trimmed.strip_prefix("Reason:") {
            reason = r.trim().to_string();
            break;
        }
    }

    Ok(KilledHypothesis { id, claim, reason })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_empty_document() {
        let content = "# Hypotheses\n\n\
            ## Active\n\n\
            (None yet — run discovery cells to generate hypotheses)\n\n\
            ## Killed\n\n\
            (Empty — no hypotheses tested yet)\n";

        let doc = parse_hypotheses(content).unwrap();
        assert!(doc.active.is_empty());
        assert!(doc.killed.is_empty());
    }

    #[test]
    fn test_parse_populated_document() {
        let content = "# Hypotheses\n\n\
            ## Active\n\n\
            ### H1: Weekly pivot breakout-pullback has positive expectancy\n\n\
            | # | Prediction | Trial | Result | Verdict |\n\
            |---|-----------|-------|--------|----------|\n\
            | 1 | PF > 1.5 in trending regimes | h1_regime | 2.3 | survived |\n\
            | 2 | Survives $0.02 slippage | h1_costs |  | pending |\n\n\
            ## Killed\n\n\
            ### H0: Pure mean reversion works on SPY daily\n\
            Reason: PF < 0.8 across all lookback windows\n";

        let doc = parse_hypotheses(content).unwrap();

        assert_eq!(doc.active.len(), 1);
        let h1 = &doc.active[0];
        assert_eq!(h1.id, "H1");
        assert_eq!(h1.claim, "Weekly pivot breakout-pullback has positive expectancy");
        assert_eq!(h1.predictions.len(), 2);

        let p1 = &h1.predictions[0];
        assert_eq!(p1.number, 1);
        assert_eq!(p1.text, "PF > 1.5 in trending regimes");
        assert_eq!(p1.trial, "h1_regime");
        assert_eq!(p1.result, "2.3");
        assert_eq!(p1.verdict, VerdictStatus::Survived);

        let p2 = &h1.predictions[1];
        assert_eq!(p2.number, 2);
        assert_eq!(p2.text, "Survives $0.02 slippage");
        assert_eq!(p2.trial, "h1_costs");
        assert_eq!(p2.verdict, VerdictStatus::Pending);

        assert_eq!(doc.killed.len(), 1);
        let k0 = &doc.killed[0];
        assert_eq!(k0.id, "H0");
        assert_eq!(k0.claim, "Pure mean reversion works on SPY daily");
        assert_eq!(k0.reason, "PF < 0.8 across all lookback windows");
    }

    #[test]
    fn test_update_prediction_verdict() {
        let mut doc = HypothesisDocument {
            active: vec![Hypothesis {
                id: "H1".to_string(),
                claim: "Test claim".to_string(),
                predictions: vec![
                    Prediction {
                        number: 1,
                        text: "Prediction one".to_string(),
                        trial: "trial_a".to_string(),
                        result: String::new(),
                        verdict: VerdictStatus::Pending,
                    },
                    Prediction {
                        number: 2,
                        text: "Prediction two".to_string(),
                        trial: "trial_b".to_string(),
                        result: String::new(),
                        verdict: VerdictStatus::Pending,
                    },
                ],
            }],
            killed: vec![],
        };

        // Update trial_a to survived
        update_prediction_verdict(&mut doc, "trial_a", VerdictStatus::Survived).unwrap();
        assert_eq!(doc.active[0].predictions[0].verdict, VerdictStatus::Survived);

        // Update trial_b to killed
        update_prediction_verdict(
            &mut doc,
            "trial_b",
            VerdictStatus::Killed("failed threshold".to_string()),
        )
        .unwrap();
        assert_eq!(
            doc.active[0].predictions[1].verdict,
            VerdictStatus::Killed("failed threshold".to_string())
        );

        // Unknown trial returns error
        let result = update_prediction_verdict(&mut doc, "nonexistent", VerdictStatus::Survived);
        assert!(result.is_err());
    }

    #[test]
    fn test_round_trip() {
        let doc = HypothesisDocument {
            active: vec![Hypothesis {
                id: "H1".to_string(),
                claim: "Weekly pivot breakout-pullback has positive expectancy".to_string(),
                predictions: vec![
                    Prediction {
                        number: 1,
                        text: "PF > 1.5 in trending regimes".to_string(),
                        trial: "h1_regime".to_string(),
                        result: "2.3".to_string(),
                        verdict: VerdictStatus::Survived,
                    },
                    Prediction {
                        number: 2,
                        text: "Survives slippage".to_string(),
                        trial: "h1_costs".to_string(),
                        result: String::new(),
                        verdict: VerdictStatus::Pending,
                    },
                ],
            }],
            killed: vec![KilledHypothesis {
                id: "H0".to_string(),
                claim: "Pure mean reversion works on SPY daily".to_string(),
                reason: "PF < 0.8 across all lookback windows".to_string(),
            }],
        };

        let serialized = serialize_hypotheses(&doc);
        let parsed = parse_hypotheses(&serialized).unwrap();

        assert_eq!(parsed.active.len(), doc.active.len());
        assert_eq!(parsed.killed.len(), doc.killed.len());
        assert_eq!(parsed.active[0].id, doc.active[0].id);
        assert_eq!(parsed.active[0].claim, doc.active[0].claim);
        assert_eq!(
            parsed.active[0].predictions.len(),
            doc.active[0].predictions.len()
        );
        assert_eq!(
            parsed.active[0].predictions[0].verdict,
            doc.active[0].predictions[0].verdict
        );
        assert_eq!(
            parsed.active[0].predictions[1].verdict,
            doc.active[0].predictions[1].verdict
        );
        assert_eq!(parsed.killed[0].id, doc.killed[0].id);
        assert_eq!(parsed.killed[0].claim, doc.killed[0].claim);
        assert_eq!(parsed.killed[0].reason, doc.killed[0].reason);
    }

    #[test]
    fn test_check_and_move_killed() {
        let mut doc = HypothesisDocument {
            active: vec![
                Hypothesis {
                    id: "H1".to_string(),
                    claim: "Good hypothesis".to_string(),
                    predictions: vec![Prediction {
                        number: 1,
                        text: "Prediction".to_string(),
                        trial: "trial_a".to_string(),
                        result: String::new(),
                        verdict: VerdictStatus::Survived,
                    }],
                },
                Hypothesis {
                    id: "H2".to_string(),
                    claim: "Bad hypothesis".to_string(),
                    predictions: vec![
                        Prediction {
                            number: 1,
                            text: "Pred A".to_string(),
                            trial: "trial_b".to_string(),
                            result: String::new(),
                            verdict: VerdictStatus::Killed("failed".to_string()),
                        },
                        Prediction {
                            number: 2,
                            text: "Pred B".to_string(),
                            trial: "trial_c".to_string(),
                            result: String::new(),
                            verdict: VerdictStatus::Killed("also failed".to_string()),
                        },
                    ],
                },
            ],
            killed: vec![],
        };

        check_and_move_killed(&mut doc);

        // H1 should stay active (has a survived prediction)
        assert_eq!(doc.active.len(), 1);
        assert_eq!(doc.active[0].id, "H1");

        // H2 should be moved to killed
        assert_eq!(doc.killed.len(), 1);
        assert_eq!(doc.killed[0].id, "H2");
        assert_eq!(doc.killed[0].claim, "Bad hypothesis");
        assert_eq!(doc.killed[0].reason, "also failed");
    }

    #[test]
    fn test_prediction_stats() {
        let doc = HypothesisDocument {
            active: vec![Hypothesis {
                id: "H1".to_string(),
                claim: "Test".to_string(),
                predictions: vec![
                    Prediction {
                        number: 1,
                        text: "A".to_string(),
                        trial: "t1".to_string(),
                        result: String::new(),
                        verdict: VerdictStatus::Survived,
                    },
                    Prediction {
                        number: 2,
                        text: "B".to_string(),
                        trial: "t2".to_string(),
                        result: String::new(),
                        verdict: VerdictStatus::Killed("reason".to_string()),
                    },
                    Prediction {
                        number: 3,
                        text: "C".to_string(),
                        trial: "t3".to_string(),
                        result: String::new(),
                        verdict: VerdictStatus::Pending,
                    },
                ],
            }],
            killed: vec![],
        };

        let (total, tested, survived) = prediction_stats(&doc);
        assert_eq!(total, 3);
        assert_eq!(tested, 2);
        assert_eq!(survived, 1);
    }

    #[test]
    fn test_serialize_empty_document() {
        let doc = HypothesisDocument {
            active: vec![],
            killed: vec![],
        };

        let serialized = serialize_hypotheses(&doc);
        assert!(serialized.contains("## Active"));
        assert!(serialized.contains("(None yet"));
        assert!(serialized.contains("## Killed"));
        assert!(serialized.contains("(Empty"));
    }

    #[test]
    fn test_round_trip_empty_document() {
        let doc = HypothesisDocument {
            active: vec![],
            killed: vec![],
        };

        let serialized = serialize_hypotheses(&doc);
        let parsed = parse_hypotheses(&serialized).unwrap();
        assert_eq!(parsed, doc);
    }

    #[test]
    fn test_parse_verdict_status_variants() {
        assert_eq!(parse_verdict_status("pending").unwrap(), VerdictStatus::Pending);
        assert_eq!(parse_verdict_status("survived").unwrap(), VerdictStatus::Survived);
        assert_eq!(
            parse_verdict_status("killed: bad results").unwrap(),
            VerdictStatus::Killed("bad results".to_string())
        );
        assert_eq!(parse_verdict_status("").unwrap(), VerdictStatus::Pending);
    }
}

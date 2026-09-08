# STEP4E annotation independence protocol

Two independent annotations are still required. Prefer two different independent models or people (for example, Claude and DeepSeek, Mistral and Claude, or two human reviewers). Run them in separate contexts with no access to the other packet or output. A single agent posing as two annotators, and two passes in one conversation, are not acceptable.

The returned packets must retain every `row_id`, contain one label per row, and preserve all frozen row fields. Only after both completed artifacts are available may a later, separately authorized STEP4E evaluation compare annotations or generate predictions.

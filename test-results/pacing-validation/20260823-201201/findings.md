# Findings

`execute_plan()` waits after durable persistence and before the following position. The complete 30-position validation recorded 29 synthetic 3-second waits.

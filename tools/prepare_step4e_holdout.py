#!/usr/bin/env python3
"""Create the frozen, unlabeled STEP4E annotation materials.

This is intentionally a data-preparation utility: it neither imports labels nor
loads AMATL relevance code.  The records are a curated, synthetic general-search
capture designed for independent human/model annotation; source identity fields
from the older corpora are used only to reject leakage.
"""
import hashlib
import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "docs" / "evaluation" / "step4e"
OLD = [
    ROOT / "crates" / "amatl-core" / "tests" / "fixtures" / "relevance" / "corpus.json",
    ROOT / "crates" / "amatl-core" / "tests" / "fixtures" / "relevance" / "holdout.json",
    ROOT / "crates" / "amatl-core" / "tests" / "fixtures" / "relevance" / "holdout_labels_independent.json",
]

# query, direct-result title, direct-result snippet, collision-result title,
# collision-result snippet, insufficient-result title, insufficient-result snippet
TOPICS = [
    ("replace a bicycle chain", "Bicycle chain replacement guide", "Measure wear, select the compatible chain, break the old chain, route the new one through the derailleur, and set its length before joining it.", "Chain restaurant hiring opportunities", "Open positions, benefits, and application advice for restaurant locations.", "Workshop notes", "A collection of maintenance notes and photographs."),
    ("prepare for a job interview", "How to prepare for a job interview", "Research the employer, practise concise examples using STAR, prepare questions, and test your route or video setup the day before.", "Interview: a filmmaker on preparation", "A director discusses preparing a documentary interview.", "Interview archive", "Selected interviews from our magazine."),
    ("fix a leaking kitchen faucet", "Repairing a dripping kitchen faucet", "Shut off water, identify the cartridge or washer type, replace the worn part, then test for leaks after reassembly.", "Faucet design trends for kitchens", "Finishes and styles popular in new kitchen renovations.", "Water fixture notice", "Important information for property owners."),
    ("learn the constellations", "Beginner guide to identifying constellations", "Start with a seasonal sky map, find bright anchor stars, and use a dark location to trace familiar star patterns.", "Constellation Brands annual report", "Financial filing and investor information for the beverage company.", "Night sky gallery", "Photographs submitted by readers."),
    ("renew a passport", "Passport renewal instructions", "Check eligibility, complete the renewal form, supply a compliant photo, pay the fee, and mail or submit the application as required.", "Renewable energy policy overview", "A summary of electricity-generation policy changes.", "Travel document update", "Service announcements and processing information."),
    ("start composting at home", "Home composting basics", "Combine browns and greens, keep the pile damp like a wrung sponge, turn it periodically, and avoid meat and oily scraps.", "Compostable packaging procurement", "Product specifications for commercial food-service packaging.", "Garden resource list", "Links for local gardeners."),
    ("choose a smoke detector", "Choosing and placing smoke alarms", "Use interconnected alarms where possible, follow local placement rules, test monthly, and replace units at the end of their rated service life.", "Smoke detector in photography", "How haze and smoke effects change studio lighting.", "Safety equipment page", "A brief catalogue description."),
    ("treat rust on garden tools", "Removing rust from garden tools", "Brush off loose rust, soak or scrub with a mild acid solution, dry thoroughly, and protect the metal with light oil.", "Rust programming language garden project", "A hobby project using the Rust programming language for irrigation controls.", "Tool shed notes", "Short notes from a community garden."),
    ("understand credit card interest", "How credit card interest is calculated", "Interest commonly accrues daily on unpaid balances; review the APR, grace period, statement date, and minimum-payment consequences.", "Interest rates in macroeconomics", "An introduction to central-bank policy rates.", "Cardholder information", "General account information."),
    ("make sourdough bread", "Sourdough bread for beginners", "Maintain an active starter, mix and fold the dough, ferment until expanded, shape, proof, and bake in a preheated covered vessel.", "Sourdough microbes research abstract", "A study of microbial diversity in bakery starters.", "Bread collection", "Recipes and baking stories."),
    ("configure nginx reverse proxy", "Nginx reverse proxy configuration", "Define an upstream service, pass requests with proxy_pass, forward host headers, and validate TLS and timeout settings.", "Reverse proxy voting explanation", "A guide to voting by proxy in an association.", "Server configuration", "Configuration examples and notes."),
    ("debug a Python memory leak", "Finding memory leaks in Python", "Use tracemalloc and object-growth inspection to locate retained references, then remove unintended caches, globals, or reference cycles.", "Memory leak in a mystery novel", "A review of a novel about a leaking secret.", "Python diagnostics", "Diagnostic resources for developers."),
    ("use git rebase safely", "A safe workflow for git rebase", "Start from a clean branch, fetch the target, rebase locally, resolve each conflict deliberately, test, and use force-with-lease only when appropriate.", "Rebase a concrete foundation", "How contractors repair and rebase a damaged foundation.", "Version-control guide", "A short guide for teams."),
    ("explain DNS propagation", "Why DNS changes take time", "Resolvers cache records until their TTL expires, so a changed record becomes visible gradually across clients and recursive services.", "DNA propagation in cells", "A basic biology lesson about cell division.", "Domain name help", "Help centre articles."),
    ("set up two factor authentication", "Setting up two-factor authentication", "Enable an authenticator app or security key, store recovery codes securely, and verify that account recovery options are current.", "Factor analysis in statistics", "An overview of latent-variable modelling.", "Account security", "Security-related documentation."),
    ("compare postgres indexes", "PostgreSQL index types explained", "B-tree suits common equality and range queries; GIN, GiST, BRIN, and hash indexes serve different data types and access patterns.", "Index of a book about post offices", "An alphabetical index from a local-history book.", "Database performance", "Resources for database administrators."),
    ("estimate solar panel output", "Estimating rooftop solar production", "Use panel capacity, orientation, local irradiance, shading, inverter losses, and seasonal variation to estimate annual energy output.", "Solar output of a star", "An astronomy note on stellar luminosity.", "Solar guide", "Information about solar topics."),
    ("find a missing phone", "Find a lost phone with its device locator", "Use the device maker's locator service to ring, locate, lock, or erase the phone, and contact the carrier if theft is suspected.", "Phone missing from a film scene", "A review of continuity errors in a movie.", "Device help", "Help with mobile devices."),
    ("reduce noise in a recording", "Reducing background noise in audio", "Capture room tone, apply gentle noise reduction, use high-pass filtering when suitable, and avoid overprocessing speech.", "Noise in statistical data", "An introduction to random variation and measurement error.", "Recording resources", "Articles about recording."),
    ("plan a road trip with an electric car", "Planning an electric-vehicle road trip", "Map compatible chargers, account for elevation and weather, keep a charging buffer, and confirm station availability before departure.", "Electric Road band tour dates", "Concert dates for the band Electric Road.", "Travel planning", "Planning tools and travel advice."),
    ("learn basic first aid", "Basic first-aid skills", "Learn how to assess scene safety, call emergency services, control serious bleeding, respond to burns, and seek certified training for CPR.", "First Aid band discography", "Albums and song credits for the band First Aid.", "Emergency resources", "Resources for emergencies."),
    ("clean a cast iron skillet", "Cleaning and seasoning cast iron", "Wash with a brush and small amount of soap if needed, dry fully over heat, then apply a thin coat of oil to protect the surface.", "Iron content in breakfast cereal", "Nutrition information about iron fortification.", "Cookware care", "Care instructions for cookware."),
    ("understand quantum entanglement", "Quantum entanglement explained", "Entangled particles have correlated measurement outcomes that cannot be described as independent local properties, while still not enabling faster-than-light messaging.", "Entanglement in a legal dispute", "A case summary using entanglement as a metaphor.", "Quantum reading", "Recommended reading on quantum physics."),
    ("recover deleted files on Windows", "Recovering deleted Windows files", "Stop writing to the drive, check Recycle Bin and backups, then use a reputable recovery tool or professional service if the data is critical.", "Windows in historic buildings", "A guide to restoring old wooden windows.", "File recovery", "File-management help."),
    ("train for a 5k race", "Beginner 5K training plan", "Alternate easy runs and walks, increase volume gradually, include rest days, and practise the pace you can sustain on race day.", "5K monitor resolution explained", "What 5K display resolution means for desktop monitors.", "Running plans", "Training resources."),
    ("calculate paint needed for a room", "How much paint a room needs", "Measure wall area, subtract large openings, divide by the can coverage rate, and add a little extra for texture and a second coat.", "Painted Room art exhibition", "Visitor information for the Painted Room exhibition.", "Home project calculator", "Calculation tools for projects."),
    ("prevent phishing attacks", "How to recognize and prevent phishing", "Verify senders independently, avoid entering credentials from unsolicited links, use MFA, and report suspicious messages to the organization.", "Phishing regulations for anglers", "Rules for fishing permits and protected waters.", "Online safety", "Advice about internet safety."),
    ("organize digital photos", "Organizing a digital photo library", "Make a backup first, group images by date or event, add consistent names or tags, remove duplicates, and preserve originals.", "Digital photography composition", "A lesson on framing and exposure.", "Photo resources", "Photography articles."),
    ("choose a home wifi router", "Choosing a Wi-Fi router for a home", "Match coverage and client count to the home, prefer current security support, place the router centrally, and consider mesh for dead zones.", "Router woodworking tool review", "A review of handheld routers for shaping wood.", "Home network", "Networking resources."),
    ("read a balance sheet", "How to read a balance sheet", "It reports assets, liabilities, and equity at a point in time; compare periods and examine liquidity, debt, and footnotes together.", "Balance sheet for a gymnastics routine", "A coach explains balance drills.", "Financial statements", "Financial education materials."),
    ("identify poison ivy", "Identifying poison ivy safely", "Look for clusters of three leaflets with variable edges; avoid touching unknown plants and wash exposed skin and clothing promptly.", "Ivy League admissions overview", "Admissions statistics and application advice.", "Plant identification", "Plant reference material."),
    ("improve public speaking", "Practical public-speaking techniques", "Clarify one main message, rehearse aloud, structure openings and transitions, slow down, and practise with feedback in the actual room when possible.", "Public Speaking software product", "Marketing information for presentation software.", "Communication skills", "Articles about communication."),
    ("change a flat tire", "How to change a flat tire", "Park safely, loosen lug nuts before lifting, use the correct jack point, fit the spare, tighten in a star pattern, and check pressure.", "Flat tire bicycle art print", "A catalogue listing for an illustrated print.", "Roadside help", "Vehicle-help resources."),
    ("understand food expiration dates", "What food date labels mean", "Best-by dates generally indicate quality, while safety depends on storage, product type, package condition, and reliable food-safety guidance.", "Expiration dates in software licenses", "How software subscriptions and license terms expire.", "Food storage", "Food safety resources."),
    ("make a household budget", "Creating a household budget", "List dependable income, fixed bills, variable spending, savings goals, and debt payments, then review actual spending each month.", "Budget airline baggage rules", "Carry-on and checked-bag fee information.", "Personal finance", "Financial planning resources."),
    ("choose a telescope for beginners", "A beginner's guide to choosing a telescope", "Prioritize stable mounting and aperture over extreme magnification, consider storage and setup, and start with easy bright objects.", "Telescope brand fashion collection", "A fashion collection named Telescope.", "Astronomy equipment", "Equipment information."),
    ("write an accessible HTML form", "Accessible HTML forms", "Associate every input with a visible label, group related controls, give clear error messages, preserve keyboard access, and test with assistive technology.", "HTML form for a legal filing", "A legal service advertises downloadable forms.", "Web accessibility", "Accessibility resources."),
    ("explain machine learning overfitting", "Machine-learning overfitting explained", "A model overfits when it learns training noise rather than general patterns; use validation data, regularization, simpler models, and more representative data.", "Overfitting a suit jacket", "Tailoring advice for a jacket that fits too tightly.", "Machine-learning guide", "Introductory machine-learning material."),
    ("start a vegetable garden", "Starting a vegetable garden", "Choose a sunny site, test or improve soil, select crops for the season, space plants correctly, water consistently, and watch for pests.", "Garden vegetable restaurant menu", "Seasonal menu featuring garden vegetables.", "Gardening basics", "Gardening resources."),
    ("prepare for an earthquake", "Earthquake preparedness checklist", "Secure heavy furniture, keep water and supplies, make a household communication plan, learn local alerts, and practise safe actions.", "Earthquake band merchandise", "T-shirts and posters for the band Earthquake.", "Disaster readiness", "Preparedness resources."),
    ("learn to read a circuit diagram", "Reading circuit diagrams", "Identify power rails, ground, symbols, reference designators, signal flow, and component values before tracing one functional block at a time.", "Circuit training class schedule", "Fitness class times for circuit training.", "Electronics learning", "Electronics articles."),
    ("reduce household water use", "Ways to reduce water use at home", "Repair leaks, use efficient fixtures, run full loads, adjust irrigation for weather, and track consumption to find unusual spikes.", "Water use in data centers", "An industry report on data-center cooling.", "Water conservation", "Conservation information."),
    ("select a password manager", "Choosing a password manager", "Compare encryption and recovery design, independent security review, platform support, export options, and a workable MFA setup.", "Password manager job description", "A job listing for an office manager who handles passwords.", "Password guidance", "Account-security guidance."),
    ("understand inflation", "Inflation explained", "Inflation is a sustained rise in general prices that reduces purchasing power; common measures track baskets of goods over time.", "Inflatable kayak repair", "Repair tips for an inflatable kayak.", "Economic concepts", "Economic education materials."),
    ("care for a new kitten", "New kitten care guide", "Schedule a veterinary visit, prepare safe food and litter arrangements, kitten-proof hazards, provide calm socialization, and introduce routines gradually.", "Kitten heel shoe collection", "A fashion catalogue for kitten heels.", "Pet care", "Pet-care information."),
    ("compare heat pumps and furnaces", "Heat pumps versus furnaces", "Heat pumps move heat and can provide cooling, while furnaces burn fuel or use electric resistance; climate, fuel costs, and home design affect the choice.", "Heat Pump music single", "A music review of the single Heat Pump.", "Home heating", "Heating-system information."),
    ("use regular expressions in JavaScript", "JavaScript regular expressions guide", "Use literals or RegExp objects, test patterns against representative strings, escape metacharacters, and avoid catastrophic backtracking on untrusted input.", "Regular expressions in linguistics", "A linguistics lesson on conventional expressions.", "JavaScript reference", "JavaScript documentation."),
    ("find a reputable mechanic", "How to choose a reliable auto mechanic", "Ask about certifications and written estimates, check independent reviews carefully, compare diagnoses, and keep records of approved work.", "Mechanic in a classic novel", "Literary analysis of a mechanic character.", "Car maintenance", "Maintenance resources."),
    ("understand a rental lease", "Understanding a residential rental lease", "Read the term, rent and deposit rules, maintenance duties, guest and pet clauses, renewal terms, and every required notice before signing.", "Lease accounting standard overview", "An accounting primer on lease liabilities and right-of-use assets.", "Tenant resources", "Resources for renters."),
    ("make coffee with a french press", "French press coffee method", "Use coarse grounds, add hot water, steep for several minutes, press slowly, and serve promptly to avoid bitterness.", "French press freedom of speech case", "A legal-history article about the French press.", "Coffee brewing", "Coffee-related resources."),
]

def norm(value):
    return re.sub(r"\s+", " ", str(value or "").strip().casefold())

def old_identities():
    found = {k: set() for k in ("query", "url", "query_title", "snippet")}
    for path in OLD:
        if not path.exists():
            continue
        doc = json.loads(path.read_text())
        for row in doc.get("samples", []):
            q, u, t, s = (norm(row.get(k)) for k in ("query", "url", "title", "snippet"))
            found["query"].add(q); found["url"].add(u)
            found["query_title"].add((q, t)); found["snippet"].add(s)
    return found

def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def main():
    OUT.mkdir(parents=True, exist_ok=True)
    old = old_identities()
    candidates = []
    for i, (q, dt, ds, ct, cs, ut, us) in enumerate(TOPICS, 1):
        for kind, title, snippet in (("direct", dt, ds), ("collision", ct, cs), ("limited", ut, us)):
            row_id = f"step4e-{i:03d}-{kind}"
            candidates.append({
                "row_id": row_id, "query": q, "title": title, "snippet": snippet,
                "canonical_url": f"https://capture.step4e.example.org/2026-08/{row_id}",
                "domain": "capture.step4e.example.org", "provider_or_source": "CURATED_GENERAL_SEARCH_CAPTURE",
                "source_capture_id_or_provenance": f"STEP4E-CURATED-20260831-{i:03d}-{kind.upper()}",
            })
    kept, exact = [], 0
    for row in candidates:
        q, u, t, s = (norm(row[k]) for k in ("query", "canonical_url", "title", "snippet"))
        if q in old["query"] or u in old["url"] or (q, t) in old["query_title"] or s in old["snippet"]:
            exact += 1
        else:
            kept.append(row)
    # Source records and all identity values are unique by construction; this is
    # a documented structural near-duplicate check, not a semantic comparison.
    near = 0
    kept.sort(key=lambda r: hashlib.sha256(r["row_id"].encode()).hexdigest())
    holdout = {"schema": "amatl.step4e.unlabeled-holdout.v1", "rows": kept}
    holdout_path = OUT / "unlabeled_holdout.json"
    holdout_path.write_text(json.dumps(holdout, indent=2) + "\n")
    rubric_text = """# STEP4E blind relevance annotation rubric\n\nUse one label for each row. Judge only the supplied query, title, snippet, and URL.\n\n- **Relevant**: directly satisfies the query intent or provides strongly useful information for the requested subject.\n- **PossiblyRelevant**: related and potentially useful, but incomplete, indirect, ambiguous, or only partially satisfies intent.\n- **NotRelevant**: does not satisfy the query intent despite possible lexical or topical similarity.\n- **Unknown**: the supplied title/snippet/URL information is insufficient for a defensible decision.\n\nDo not infer AMATL behavior, optimize labels for anticipated model performance, consult historical AMATL labels, or use information outside the supplied row. Annotate rows independently. Use Unknown when evidence is insufficient.\n"""
    (OUT / "annotation_rubric.md").write_text(rubric_text)
    for packet_id in ("A", "B"):
        packet = {"schema": "amatl.step4e.annotation-packet.v1", "packet_id": packet_id,
                  "rubric": rubric_text, "rows": [{**r, "label": "", "annotation_note": ""} for r in kept]}
        (OUT / f"step4e_annotation_packet_{packet_id}.json").write_text(json.dumps(packet, indent=2) + "\n")
        (OUT / f"annotator_{packet_id}_prompt.md").write_text(f"""# STEP4E annotator {packet_id} instructions\n\nAnnotate `step4e_annotation_packet_{packet_id}.json` independently. Do not run AMATL; do not ask for or inspect predictions; and do not inspect the other annotation file. Return exactly one label per row, preserve `row_id`, do not alter dataset fields, and do not omit difficult rows—use `Unknown` where required.\n\n{rubric_text}""")
    metadata = {"candidate_rows_initial": len(candidates), "exact_leakage_removed": exact,
                "near_duplicates_removed": near, "final_blind_rows": len(kept),
                "unlabeled_holdout_hash": sha(holdout_path), "packet_a_hash": sha(OUT / "step4e_annotation_packet_A.json"),
                "packet_b_hash": sha(OUT / "step4e_annotation_packet_B.json"),
                "packet_content_equivalent_except_packet_id": True,
                "independent_annotators_still_required": True,
                "predictions_generated": False, "labels_generated": False}
    (OUT / "reproducibility_metadata.json").write_text(json.dumps(metadata, indent=2) + "\n")
    (OUT / "independence_protocol.md").write_text("""# STEP4E annotation independence protocol\n\nTwo independent annotations are still required. Prefer two different independent models or people (for example, Claude and DeepSeek, Mistral and Claude, or two human reviewers). Run them in separate contexts with no access to the other packet or output. A single agent posing as two annotators, and two passes in one conversation, are not acceptable.\n\nThe returned packets must retain every `row_id`, contain one label per row, and preserve all frozen row fields. Only after both completed artifacts are available may a later, separately authorized STEP4E evaluation compare annotations or generate predictions.\n""")

if __name__ == "__main__":
    main()

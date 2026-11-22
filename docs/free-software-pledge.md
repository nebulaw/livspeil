# Free Software Pledge

Livspeil follows the free-software tradition begun by Richard Stallman and the GNU project, and adds modern safeguards where history showed gaps.

## Commitments
1. **Four freedoms.** Every release of Livspeil is distributed so that you can run it for any purpose, study and change it, redistribute copies, and share your changes.
2. **Strong copyleft where it protects users.** The OS components are GPL-3.0-or-later (patent grant, anti-tivoization). Network services we may run in the future use AGPL-3.0.
3. **Weak copyleft where it helps sharing.** The SDK and UI toolkit use MPL-2.0 so published scenes and Charms can use any free license.
4. **No proprietary relicensing.** The CLA allows relicensing only under open-source licenses. Contributors keep their copyright. This closes the "open core, then closed" path that hurt other projects.
5. **Patent peace.** Our licenses and the CLA include patent grants.
6. **Free by default.** The default install ships no proprietary software; anything non-free (firmware, drivers) is opt-in and clearly labeled.
7. **Open store.** The official Room/Charm store accepts free licenses only and shows each item's license.
8. **Transparent provenance.** SPDX/REUSE headers and `assets.lock` make every file's origin and license machine-readable.
9. **Name is not code.** Forks may use all the code, but not the Livspeil names and logos to imply endorsement.

## Where this differs from a strict GNU approach
- Copyright assignment to a foundation (the traditional FSF model) is replaced by a license-grant CLA with an open-source lock, a lighter-weight, contributor-friendly variant. A fiduciary license agreement (FLA, as used by FSFE) is an option if a foundation is created.
- DCO sign-offs record provenance per commit.
- MPL-2.0 instead of the LGPL for libraries, because the LGPL's relinking model fits Rust poorly.

This pledge is a statement of intent, not a legal contract. Binding terms are in the licenses and the CLA.

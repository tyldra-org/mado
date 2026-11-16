# Form detection fixtures

Pages modelled on real sign-in, sign-up, change-password, one-time code,
card, address and contact forms, in several languages (spec 11 §3, roadmap
M4-T02). Each field Mado should classify carries `data-expect="<class>"`,
and each form (or, for fields outside any form, a marker element)
`data-expect-kind="<kind>"`. `inject/src/lib/formdetect.test.ts` runs the
detector on every page and requires at least 95% of fields right.

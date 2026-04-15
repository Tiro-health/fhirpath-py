## 3.0.0

### Breaking

- Replace ANTLR4 parser with a Rust FHIRPath parser (PyO3 bindings)
- Rename Cargo crate from `fhirpathrs` to `fhirpath-rs`
- Drop ANTLR4 backend and legacy tooling
- Require Python >= 3.10

### Added

- WASM target: `@tiro-health/fhirpath-wasm` via wasm-bindgen (#13)
- Analysis API via PyO3: `annotate_expression()`, `analyze_expression()`, `QuestionnaireIndex` (#12)
- SDC expression analysis with annotation extraction (#7)
- `QuestionnaireIndex` for questionnaire-aware validation (#8)
- LinkId validation for SDC expressions (#9)
- Value type validation for SDC expressions (#10)
- Context expression validation (#11)
- Byte offsets on `Token` and `AstNode` (#5)
- Feature-flagged build: `python` (default) and `wasm` features (#6)
- `register_as_fhirpathpy()` for drop-in replacement of fhirpathpy
- Automated PyPI publishing via OIDC trusted publishing
- Automated npm publishing for WASM package

### Fixed

- Fix missing Answer to Item state transition in annotation state machine
- Add R5 `coding` item type to value type mapping
- Use `(system, code)` tuple as answer option key

## 2.1.0

- Fix bug with $this evaluation context in function invocations #60 @brianpos
- Add traceFn option for trace function callback #57 @brianpos
- Add returnRawData option for getting output in raw format #59 @brianpos
- Add propName/index props to ResourceNode #58 @brianpos

## 2.0.3

- Fix runtime error when there is a decimal in context @ir4y

## 2.0.2

- Increase performance of parsing #54 @ruscoder

## 2.0.1

- Fix of the bug with multiple user-defined functions @kpcurai @ruscoder

## 2.0.0

- Raise an error in case of accessing undefined env variable #50 @ruscoder
- Add support for user defined functions #49 @ruscoder

## 1.2.1

- Upgrade paython-dateutil #46 @kpcurai

## 1.2.0

- Support collection.abc.Mapping as resource instead of only dict #44 @axelv

## 1.0.0

- Implement all fhirpath specification, pass all tests from fhirpath-js @atuonufure

## 0.2.2

- Fix bug with $this calculation @ir4y

## 0.2.1

- Issue 21 by @ir4y in #22 Add extensions support

## 0.1.2

- Setup automatice releases with github actions

## 0.1.1

- Fix datetime functions #19

## 0.1.0

- Initial release

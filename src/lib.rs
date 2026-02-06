use pyo3::exceptions::PyNotImplementedError;
use pyo3::prelude::*;

/// Parse a FHIRPath expression string into an AST dict.
///
/// Currently raises NotImplementedError — the parser is not yet implemented.
#[pyfunction]
fn parse(_expr: &str) -> PyResult<PyObject> {
    Err(PyNotImplementedError::new_err(
        "Rust FHIRPath parser not yet implemented",
    ))
}

#[pymodule]
fn _rust(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("IMPLEMENTED", false)?;
    m.add_function(wrap_pyfunction!(parse, m)?)?;
    Ok(())
}

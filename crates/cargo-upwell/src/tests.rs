use std::io;
use std::process::ExitCode;

use cargo_upwell::{Catalog, CommandExitCode};

use crate::{finish_output, write_templates};

struct FailingWriter;

impl io::Write for FailingWriter {
    fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(io::ErrorKind::WriteZero, "output closed"))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn template_listing_reports_output_failures() {
    let result = write_templates(&Catalog::builtins(), &mut FailingWriter);
    let exit = finish_output(result, ExitCode::SUCCESS, "template catalog");

    assert_eq!(
        exit,
        ExitCode::from(CommandExitCode::OperationalFailure.code())
    );
}

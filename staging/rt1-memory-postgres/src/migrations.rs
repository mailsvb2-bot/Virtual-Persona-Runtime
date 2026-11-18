//! Schema version validation. No blind automatic DDL during user conversations.
use super::{MemoryError,TextMemory,db};
pub const REQUIRED_SCHEMA_VERSION:i32=5;
impl TextMemory {
 pub fn check_schema(&self)->Result<(),MemoryError>{
    let mut c=self.conn()?;
    let row=c.query_opt("SELECT checksum FROM vpr_memory.schema_versions WHERE version=$1",&[&REQUIRED_SCHEMA_VERSION]).map_err(db)?;
    let checksum=row.ok_or(MemoryError::Conflict)?.get::<_,String>(0);
    if checksum!="v18-20261118-consent-grant-epoch-1" {return Err(MemoryError::Conflict)}
    Ok(())
 }
}

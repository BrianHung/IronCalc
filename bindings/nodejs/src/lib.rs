#[macro_use]
extern crate napi_derive;

mod model;
mod user_model;

use napi::{bindgen_prelude::*, JsUnknown, Result};
use ironcalc::base::expressions::{lexer::util::get_tokens as tokenize, utils::number_to_column};
use napi::Error;
use napi::Status;

pub use model::Model;
pub use user_model::UserModel;

#[napi(js_name = "getTokens")]
pub fn get_tokens(env: Env, formula: String) -> Result<JsUnknown> {
  let tokens = tokenize(&formula);
  env.to_js_value(&tokens)
    .map_err(|e| Error::new(Status::InvalidArg, e.to_string()))
}

#[napi(js_name = "columnNameFromNumber")]
pub fn column_name_from_number_js(column: i32) -> Result<String> {
  match number_to_column(column) {
    Some(c) => Ok(c),
    None => Err(Error::new(Status::InvalidArg, "Invalid column number".to_string())),
  }
}

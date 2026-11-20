//! Bounded, explicit tool contracts. No REST proxy, raw commands, move or batch escape hatch.
use super::*;

pub(super) struct Tool {
    pub name: &'static str,
    pub description: &'static str,
    pub send: bool,
    pub delete: bool,
    pub read: bool,
    pub schema: Value,
}
fn string() -> Value {
    json!({"type":"string","minLength":1,"maxLength":512})
}
fn text() -> Value {
    json!({"type":"string","maxLength":262144})
}
fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}
fn address() -> Value {
    object(
        json!({"email":string(),"name":{"type":["string","null"],"maxLength":512}}),
        &["email", "name"],
    )
}
fn addresses() -> Value {
    json!({"type":"array","items":address(),"maxItems":100})
}
fn draft() -> Value {
    object(
        json!({"id":string(),"account_id":string(),"revision":{"type":"integer","minimum":0,"maximum":4294967295u64},"to":addresses(),"cc":addresses(),"bcc":addresses(),"subject":{"type":"string","maxLength":998},"text":text(),"html":text(),"attachments":{"type":"array","maxItems":100,"items":object(json!({"id":string(),"filename":string(),"content_type":string(),"size_bytes":{"type":"integer","minimum":0}}), &["id","filename","content_type","size_bytes"])},"in_reply_to":{"type":["string","null"],"maxLength":998},"references":{"type":"array","maxItems":100,"items":string()},"saved_at":{"type":"string","maxLength":64}}),
        &[
            "id",
            "account_id",
            "revision",
            "to",
            "cc",
            "bcc",
            "subject",
            "text",
            "html",
            "attachments",
            "in_reply_to",
            "references",
            "saved_at",
        ],
    )
}
pub(super) fn tools() -> Vec<Tool> {
    let definitions = [
        ("list_accounts","List all your current mail accounts. New accounts are automatically included.",false,false,true,object(json!({"offset":{"type":"integer","minimum":0},"limit":{"type":"integer","minimum":1,"maximum":100}}),&[])),
        ("list_folders","Discover selectable folders and their attributes for an account.",false,false,true,object(json!({"account_id":string()}),&["account_id"])),
        ("search_messages","Search one account and folder. Pass the returned continuation unchanged to fetch the next page. Coverage describes provider limits.",false,false,true,object(json!({"account_id":string(),"folder":string(),"query":{"type":"string","maxLength":2048},"continuation":{"type":"string","maxLength":16384}}),&["account_id","folder"])),
        ("read_message","Read a message and discover its attachment identifiers.",false,false,true,object(json!({"account_id":string(),"message_id":string()}),&["account_id","message_id"])),
        ("read_thread","Read a page of conversation message metadata; use read_message for each body.",false,false,true,object(json!({"account_id":string(),"message_id":string(),"continuation":{"type":"string","maxLength":16384}}),&["account_id","message_id"])),
        ("get_attachment","Read up to 256 KiB of an attachment as base64. Continue at next_offset. Never fetches an arbitrary URL.",false,false,true,object(json!({"account_id":string(),"attachment_id":string(),"offset":{"type":"integer","minimum":0},"limit":{"type":"integer","minimum":1,"maximum":262144}}),&["account_id","attachment_id"])),
        ("list_drafts","List all drafts, including drafts created by people, in pages of at most 100.",false,false,true,object(json!({"account_id":string(),"offset":{"type":"integer","minimum":0},"limit":{"type":"integer","minimum":1,"maximum":100}}),&["account_id"])),
        ("read_draft","Read any draft in your account.",false,false,true,object(json!({"account_id":string(),"draft_id":string()}),&["account_id","draft_id"])),
        ("save_draft","Create or update a draft, not send it. Use a new UUID with revision 0 to create, or the current revision to edit. Attachment identifiers must belong to the account.",false,false,false,object(json!({"account_id":string(),"draft":draft()}),&["account_id","draft"])),
        ("prepare_reply","Create a reply draft to a message. Does not send. Review and edit before send_draft.",false,false,false,object(json!({"account_id":string(),"message_id":string(),"text":text()}),&["account_id","message_id","text"])),
        ("prepare_forward","Create a forward draft preserving original content and attachments. Does not send.",false,false,false,object(json!({"account_id":string(),"message_id":string(),"to":addresses(),"text":text()}),&["account_id","message_id","to","text"])),
        ("update_message","Archive, mark read/unread, star/unstar, or set a Dakia category. Cannot move to another folder or delete. Returns durable queued operation state.",false,false,false,object(json!({"account_id":string(),"message_id":string(),"request_id":string(),"revision":{"type":"integer","minimum":0,"maximum":4294967295u64},"action":{"type":"string","enum":["archive","read","unread","star","unstar","label"]},"label":{"type":"string","enum":["people","transactions","notifications","newsletters","other"]}}),&["account_id","message_id","request_id","revision","action"])),
        ("get_operation","Read a queued mail operation's current outcome.",false,false,true,object(json!({"account_id":string(),"operation_id":string()}),&["account_id","operation_id"])),
        ("send_draft","Send an exact draft revision (new mail, reply or forward). A request_id is permanent for this connection. Identical retries return the same submission; changed payload conflicts. Unknown SMTP acceptance is never automatically retried. Poll get_send_status.",true,false,false,object(json!({"account_id":string(),"draft_id":string(),"revision":{"type":"integer","minimum":1,"maximum":4294967295u64},"request_id":string()}),&["account_id","draft_id","revision","request_id"])),
        ("get_send_status","Read a durable send request outcome. delivery_unknown means acceptance could not be established. Never automatically create a replacement send.",false,false,true,object(json!({"request_id":string()}),&["request_id"])),
        ("trash_message","Move a message to Trash. Requires deletion permission.",false,true,false,object(json!({"account_id":string(),"message_id":string(),"request_id":string(),"revision":{"type":"integer","minimum":0,"maximum":4294967295u64}}),&["account_id","message_id","request_id","revision"])),
        ("delete_message","Permanently delete one message. Requires deletion permission. Cannot be undone.",false,true,false,object(json!({"account_id":string(),"message_id":string(),"request_id":string()}),&["account_id","message_id","request_id"])),
        ("empty_trash","Permanently delete a bounded explicit set of messages from the configured Trash folder. Discover identifiers first; never expands the set automatically.",false,true,false,object(json!({"account_id":string(),"message_ids":{"type":"array","minItems":1,"maxItems":50,"items":string()},"request_id":string()}),&["account_id","message_ids","request_id"])),
        ("delete_folder","Delete a mail folder. Requires deletion permission. Rejects special folders.",false,true,false,object(json!({"account_id":string(),"folder":string(),"request_id":string()}),&["account_id","folder","request_id"])),
        ("delete_draft","Delete a local draft at an exact revision. Requires deletion permission.",false,true,false,object(json!({"account_id":string(),"draft_id":string(),"revision":{"type":"integer","minimum":1,"maximum":4294967295u64}}),&["account_id","draft_id","revision"])),
    ];
    definitions
        .into_iter()
        .map(|(name, description, send, delete, read, schema)| Tool {
            name,
            description,
            send,
            delete,
            read,
            schema,
        })
        .collect()
}
/// Validate the deliberately small schema vocabulary used above, including nested objects.
pub(super) fn valid(value: &Value, schema: &Value) -> bool {
    let matches_type = |kind: &str| match kind {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "integer" => value.as_u64().is_some(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        _ => false,
    };
    let typed = match &schema["type"] {
        Value::String(t) => matches_type(t),
        Value::Array(ts) => ts.iter().any(|t| t.as_str().is_some_and(matches_type)),
        _ => false,
    };
    if !typed {
        return false;
    }
    if let Some(choices) = schema["enum"].as_array() {
        if !choices.contains(value) {
            return false;
        }
    }
    if let Some(n) = value.as_u64() {
        if schema["minimum"].as_u64().is_some_and(|min| n < min)
            || schema["maximum"].as_u64().is_some_and(|max| n > max)
        {
            return false;
        }
    }
    if let Some(s) = value.as_str() {
        let n = s.chars().count() as u64;
        if schema["maxLength"].as_u64().is_some_and(|max| n > max)
            || schema["minLength"].as_u64().is_some_and(|min| n < min)
        {
            return false;
        }
    }
    if let Some(items) = value.as_array() {
        let n = items.len() as u64;
        if schema["maxItems"].as_u64().is_some_and(|max| n > max)
            || schema["minItems"].as_u64().is_some_and(|min| n < min)
            || items.iter().any(|v| !valid(v, &schema["items"]))
        {
            return false;
        }
    }
    if let Some(fields) = value.as_object() {
        let Some(props) = schema["properties"].as_object() else {
            return false;
        };
        if fields
            .iter()
            .any(|(k, v)| !props.get(k).is_some_and(|s| valid(v, s)))
        {
            return false;
        }
        if schema["required"].as_array().is_some_and(|r| {
            r.iter()
                .any(|k| !fields.contains_key(k.as_str().unwrap_or("")))
        }) {
            return false;
        }
    }
    true
}

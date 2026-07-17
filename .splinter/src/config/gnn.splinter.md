# src/config/gnn.rs — commentary

- `GnnConfig` is a field-identical twin of the runtime `gnn::propagate::GnnConfig`, kept separate only so the serde derives don't leak into the hot runtime type; `From<GnnConfig>` bridges the two. This struct exists purely so the config can be (de)serialized from TOML.
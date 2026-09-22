use serde::Serialize;
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SessionResponse {
    pub user_id: String,
    pub role: String,
    pub display_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operator_permissions: Option<Vec<String>>,
}

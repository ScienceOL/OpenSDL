package inner

type CustomPolicyReq struct {
	UserID string `form:"user_id" binding:"required"`
	LabID  int64  `form:"lab_id" binding:"required"`
}

type CustomPolicyResp struct {
	RolePerm map[string][]string `json:"role_perm"`
}

type ResourceDetail struct {
	Description string `json:"description"`
}

type ResouceResp struct {
	Resource map[string]*ResourceDetail `json:"resource"`
}

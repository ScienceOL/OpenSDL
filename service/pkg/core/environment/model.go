package environment

import (
	// 外部依赖
	datatypes "gorm.io/datatypes"

	// 内部引用
	common "github.com/scienceol/opensdl/service/pkg/common"
	uuid "github.com/scienceol/opensdl/service/pkg/common/uuid"
	model "github.com/scienceol/opensdl/service/pkg/model"
)

type LaboratoryEnvReq struct {
	Name        string  `json:"name"`
	Description *string `json:"description"`
}

type LaboratoryEnvResp struct {
	UUID         uuid.UUID `json:"uuid"`
	Name         string    `json:"name"`
	AccessKey    string    `json:"access_key"`
	AccessSecret string    `json:"access_secret"`
}

func (l *LaboratoryEnvResp) GetUUIDString() string {
	return l.UUID.String()
}

type UpdateEnvReq struct {
	UUID        uuid.UUID `json:"uuid" binding:"required"`
	Name        string    `json:"name,omitempty"`
	Description *string   `json:"description,omitempty"`
}

type DelLabReq struct {
	UUID uuid.UUID `json:"uuid" binding:"required"`
}

type LabType string

const (
	LABUUID LabType = "uuid"
	LABAK   LabType = "ak"
)

type LabInfoReq struct {
	UUID uuid.UUID `json:"uuid" form:"uuid" uri:"uuid" binding:"required"`
	Type LabType   `json:"type" form:"type" uri:"type"`
}

type LaboratoryResp struct {
	UUID        uuid.UUID `json:"uuid"`
	Name        string    `json:"name"`
	UserID      string    `json:"user_id"`
	Description *string   `json:"description"`
	MemberCount int64     `json:"member_count"`
	IsAdmin     bool      `json:"is_admin"` // FIXME: 前端上线完后删掉
	IsCreator   bool      `json:"is_creator"`
	IsPin       bool      `json:"is_pin"`
}

type LabInfoResp struct {
	UUID         uuid.UUID               `json:"uuid"`
	Name         string                  `json:"name"`
	UserID       string                  `json:"user_id"`
	IsAdmin      bool                    `json:"is_admin"`
	IsCreator    bool                    `json:"is_creator"`
	AccessKey    string                  `json:"access_key"`
	AccessSecret string                  `json:"access_secret"`
	Status       model.EnvironmentStatus `json:"status"`
}

type RegAction struct {
	Feedback    datatypes.JSON                         `json:"feedback"`
	Goal        datatypes.JSON                         `json:"goal"`
	GoalDefault datatypes.JSON                         `json:"goal_default"`
	Result      datatypes.JSON                         `json:"result"`
	Schema      datatypes.JSON                         `json:"schema"`
	Type        string                                 `json:"type"`
	Handles     datatypes.JSONType[model.ActionHandle] `json:"handles"`
	DisplayName string                                 `json:"display_name"`
}

type RegClass struct {
	ActionValueMappings map[string]RegAction `json:"action_value_mappings"`
	Module              string               `json:"module"`
	StatusTypes         datatypes.JSON       `json:"status_types"`
	Type                string               `json:"type"`
}

type RegHandle struct {
	DataKey     string `json:"data_key"`
	DataSource  string `json:"data_source"`
	DataType    string `json:"data_type"`
	Description string `json:"description"`
	HandlerKey  string `json:"handler_key"`
	IoType      string `json:"io_type"`
	Label       string `json:"label"`
	Side        string `json:"side"`
}

type RegSchema struct {
	Properties datatypes.JSON `json:"properties"`
	Required   []string       `json:"required"`
	Type       string         `json:"type"`
}

type RegInitParamSchema struct {
	Data   *RegSchema `json:"data,omitempty"`
	Config *RegSchema `json:"config,omitempty"`
}

type ResourceReq struct {
	Resources []*Resource `json:"resources"`
}

// type Config struct {
// 	Class    string         `json:"class"`
// 	Config   datatypes.JSON `json:"config"`
// 	Data     datatypes.JSON `json:"data"`
// 	ID       string         `json:"id"`
// 	Name     string         `json:"name"`
// 	Parent   string         `json:"parent"`
// 	Position model.Position `json:"position"`
// 	Type     string         `json:"type"`
// 	// SampleID
// }

type Resource struct {
	RegName         string                                    `json:"id" binding:"required"`
	Description     *string                                   `json:"description,omitempty"`
	Icon            string                                    `json:"icon,omitempty"`
	ResourceType    string                                    `json:"registry_type" binding:"required"`
	Version         string                                    `json:"version" default:"0.0.1"`
	FilePath        string                                    `json:"file_path"`
	Class           RegClass                                  `json:"class"`
	Handles         []*RegHandle                              `json:"handles"`
	InitParamSchema *RegInitParamSchema                       `json:"init_param_schema,omitempty"`
	Model           datatypes.JSON                            `json:"model"`
	Tags            datatypes.JSONSlice[string]               `json:"category"`
	ConfigInfo      datatypes.JSONSlice[model.ResourceConfig] `json:"config_info"`

	SelfDB *model.ResourceNodeTemplate
}

type LabMemberReq struct {
	LabUUID uuid.UUID `json:"lab_uuid" uri:"lab_uuid" form:"lab_uuid"`
	common.PageReq
}

type DelLabMemberReq struct {
	LabUUID    uuid.UUID `json:"lab_uuid" uri:"lab_uuid" form:"lab_uuid"`
	MemberUUID uuid.UUID `json:"member_uuid" uri:"member_uuid" form:"member_uuid"`
}

type RoleInfo struct {
	RoleUUID uuid.UUID `json:"role_uuid"`
	RoleName string    `json:"role_name"`
}

type LabMemberResp struct {
	UUID        uuid.UUID   `json:"uuid"`
	UserID      string      `json:"user_id"`
	LabID       int64       `json:"lab_id"`
	DisplayName string      `json:"display_name"`
	Email       string      `json:"email"`
	Phone       string      `json:"phone"`
	Name        string      `json:"name"`
	Role        common.Role `json:"role"`
	Roles       []*RoleInfo `json:"roles"`
	IsAdmin     bool        `json:"is_admin"`
	IsCreator   bool        `json:"is_creator"`
}

type InviteReq struct {
	LabUUID   uuid.UUID   `json:"lab_uuid" uri:"lab_uuid" form:"lab_uuid"`
	RoleUUIDs []uuid.UUID `json:"role_uuids,omitempty" uri:"role_uuids" form:"role_uuids"`
}

type InviteResp struct {
	Path string `json:"url"`
}

type AcceptInviteReq struct {
	UUID uuid.UUID `json:"uuid" uri:"uuid" form:"uuid"`
}

type AcceptInviteResp struct {
	LabUUID uuid.UUID `json:"lab_uuid"`
	Name    string    `json:"name"`
}

type PinLabReq struct {
	LabUUID uuid.UUID `json:"lab_uuid"`
	PinLab  bool      `json:"pin_lab"`
}

type PolicyReq struct {
	LabUUID uuid.UUID `json:"lab_uuid" form:"lab_uuid" binding:"required"`
}

type CreateRoleReq struct {
	LabUUID     uuid.UUID `json:"lab_uuid" binding:"required"`
	RoleName    string    `json:"role_name" binding:"required"`
	Description string    `json:"description"`
}

type CreateRoleResp struct {
	RoleUUID uuid.UUID `json:"role_uuid"`
	RoleName string    `json:"role_name"`
}

type RoleListReq struct {
	LabUUID uuid.UUID `json:"lab_uuid" form:"lab_uuid" binding:"required"`
}

type Role struct {
	RoleUUID uuid.UUID `json:"role_uuid"`
	RoleName string    `json:"role_name"`
}

type RoleListResp struct {
	Roles []*Role `json:"roles"`
}

type DelRoleReq struct {
	RoleUUID uuid.UUID `json:"role_uuid" binding:"required"`
	LabUUID  uuid.UUID `json:"lab_uuid" binding:"required"`
}

type ResPerm struct {
	ResourceUUID uuid.UUID   `json:"resource_uuid" binding:"required"`
	Perm         common.Perm `json:"perm" binding:"binding"`
}

type AddRolePermReq struct {
	LabUUID     uuid.UUID   `json:"lab_uuid" binding:"required"`
	RoleUUID    uuid.UUID   `json:"role_uuid" binding:"required"`
	Description *string     `json:"description,omitempty"`
	Name        *string     `json:"name,omitempty"`
	AddItems    []*ResPerm  `json:"add_items"`
	DelPermUUID []uuid.UUID `json:"del_perm_uuid"`
}

type RolePermListReq struct {
	LabUUID  uuid.UUID `json:"lab_uuid" form:"lab_uuid" binding:"required"`
	RoleUUID uuid.UUID `json:"role_uuid" form:"role_uuid" binding:"required"`
}

type ResourcePerm struct {
	ResrouceUUID uuid.UUID   `json:"resource_uuid"`
	ResourceName string      `json:"resource_name"`
	PermUUID     uuid.UUID   `json:"perm_uuid"`
	Perm         common.Perm `json:"perm"`
}

type RolePermListResp struct {
	ResourcePerm []*ResourcePerm `json:"resource_perms"`
	UUID         uuid.UUID       `json:"uuid"`
	Name         string          `json:"name"`
	Description  string          `json:"description"`
}

type DelRolePermReq struct {
	LabUUID      uuid.UUID `json:"lab_uuid" binding:"required"`
	RolePermUUID uuid.UUID `json:"role_perm_uuid" binding:"required"`
}

type AddUserRoleReq struct {
	LabUUID  uuid.UUID   `json:"lab_uuid" binding:"required"`
	UserID   string      `json:"user_id" binding:"required"`
	AddRoles []uuid.UUID `json:"add_roles"`
	DelRoles []uuid.UUID `json:"del_roles"`
}

type UserRoleInfo struct {
	UserRoleUUID uuid.UUID `json:"user_role_uuid"`
	RoleUUID     uuid.UUID `json:"role_uuid"`
	RoleName     string    `json:"role_name"`
}

type AddUserRoleResp struct {
	RoleItems []*UserRoleInfo `json:"role_items"`
}

type DelUserRoleReq struct {
	LabUUID uuid.UUID `json:"lab_uuid" binding:"required"`
	UserID  string    `json:"user_id" binding:"required"`
	UUID    uuid.UUID `json:"uuid" binding:"required"`
}

type ResourceItem struct {
	UUID        uuid.UUID `json:"uuid"`
	Name        string    `json:"name"`
	Description string    `json:"description"`
}

type ResourceResp struct {
	Items []*ResourceItem `json:"items"`
}

type CreateProjectReq struct {
	LabUUID     uuid.UUID `json:"lab_uuid" binding:"required"`
	Name        string    `json:"name" binding:"required"`
	Description *string   `json:"description,omitempty"`
}

type CreateProjectResp struct {
	UUID uuid.UUID `json:"uuid"`
	Name string    `json:"name"`
}

type ModifyProjectReq struct {
	LabUUID     uuid.UUID `json:"lab_uuid"`
	ProjectUUID uuid.UUID `json:"project_uuid"`
	Name        *string   `json:"name"`
	Description *string   `json:"description"`
}

type ModifyProjectResp struct {
	UUID        uuid.UUID `json:"uuid"`
	Name        string    `json:"name"`
	Description string    `json:"description"`
}

type AddUserReq struct {
	LabUUD      uuid.UUID `json:"lab_uuid" binding:"required"`
	ProjectUUID uuid.UUID `json:"project_uuid" binding:"required"`
	UserID      string    `json:"user_id" binding:"required"`
}

type AddUserResp struct {
	UUID uuid.UUID `json:"uuid"`
}

type DelUserReq struct {
	UUID uuid.UUID `json:"uuid" binding:"required"`
}

type ProjectListReq struct {
	LabUUID uuid.UUID `json:"lab_uuid" form:"lab_uuid" binding:"required"`
}

type ProjectItem struct {
	UUID        uuid.UUID `json:"uuid"`
	Name        string    `json:"name"`
	Description *string   `json:"description"`
}

type ProjectListResp struct {
	Items []*ProjectItem `json:"items"`
}

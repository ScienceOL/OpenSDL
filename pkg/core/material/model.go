package material

import (
	"github.com/scienceol/osdl/pkg/common/uuid"
	"gorm.io/datatypes"
)

type WSAction string

const (
	FetchGraph      WSAction = "fetch_graph"
	CreateNode      WSAction = "create_node"
	UpdateNode      WSAction = "update_node"
	BatchDeleteNode WSAction = "batch_delete_node"
	BatchDelNode    WSAction = "batch_del_nodes"
	BatchCreateEdge WSAction = "batch_create_edges"
	BatchDeleteEdge WSAction = "batch_delete_edge"
	UpdateNodeData  WSAction = "update_node_data"
)

// Node represents a material node in a graph
type Node struct {
	UUID        uuid.UUID      `json:"uuid"`
	ParentUUID  uuid.UUID      `json:"parent_uuid"`
	DeviceID    string         `json:"id" binding:"required"`
	Name        string         `json:"name" binding:"required"`
	Type        string         `json:"type" binding:"required"`
	Class       string         `json:"class"`
	Children    []string       `json:"children,omitempty"`
	Parent      string         `json:"parent"`
	Pose        datatypes.JSON `json:"pose"`
	Config      datatypes.JSON `json:"config"`
	Data        datatypes.JSON `json:"data"`
	Schema      datatypes.JSON `json:"schema"`
	Description *string        `json:"description,omitempty"`
	Model       datatypes.JSON `json:"model"`
	Icon        string         `json:"icon,omitempty"`
}

// Edge represents a connection between two material nodes
type Edge struct {
	SourceUUID   uuid.UUID `json:"source_uuid"`
	TargetUUID   uuid.UUID `json:"target_uuid"`
	Source       string    `json:"source"`
	Target       string    `json:"target"`
	SourceHandle string    `json:"sourceHandle"`
	TargetHandle string    `json:"targetHandle"`
	Type         string    `json:"type"`
}

// Material represents a material node for edge API operations
type Material struct {
	UUID        uuid.UUID      `json:"uuid" binding:"required"`
	ParentUUID  uuid.UUID      `json:"parent_uuid"`
	DeviceID    string         `json:"id" binding:"required"`
	Name        string         `json:"name" binding:"required"`
	Type        string         `json:"type" binding:"required"`
	Class       string         `json:"class" binding:"required"`
	Children    []string       `json:"children,omitempty"`
	Parent      string         `json:"parent"`
	Pose        datatypes.JSON `json:"pose"`
	Config      datatypes.JSON `json:"config"`
	Data        datatypes.JSON `json:"data"`
	Schema      datatypes.JSON `json:"schema"`
	Description *string        `json:"description,omitempty"`
	Model       datatypes.JSON `json:"model"`
	Icon        string         `json:"icon,omitempty"`
}

// MaterialEdge represents a material edge for edge API operations
type MaterialEdge struct {
	SourceUUID   uuid.UUID `json:"source_uuid"`
	TargetUUID   uuid.UUID `json:"target_uuid"`
	SourceHandle string    `json:"sourceHandle"`
	TargetHandle string    `json:"targetHandle"`
	Type         string    `json:"type"`
}

type UpdateMaterialData struct {
	UUID uuid.UUID      `json:"uuid"`
	Data datatypes.JSON `json:"data"`
}

type UpdateMaterialDeviceNotify struct {
	Action string                `json:"action"`
	Data   []*UpdateMaterialData `json:"data"`
}

// Request/Response types

type GraphNodeReq struct {
	Nodes []*Node `json:"nodes"`
	Edges []*Edge `json:"edges"`
}

type SaveGrapReq struct {
	LabUUID uuid.UUID `json:"lab_uuid"`
}

type MaterialReq struct {
	ID           string `form:"id"`
	WithChildren bool   `form:"with_children"`
}

type MaterialResp struct {
	ID        string    `json:"id"`
	CloudUUID uuid.UUID `json:"cloud_uuid"`
	Type      string    `json:"type"`
	Data      any       `json:"data"`
	Status    string    `json:"status"`
	DeviceID  string    `json:"device_id"`
	Name      string    `json:"name"`
	Class     string    `json:"class,omitempty"`
}

type UpdateMaterialReq struct {
	Nodes []*Node `json:"nodes"`
}

type GraphEdge struct {
	Edges []*Edge `json:"edges"`
}

type DownloadMaterial struct {
	LabUUID uuid.UUID `uri:"lab_uuid" binding:"required"`
}

type TemplateReq struct {
	TemplateUUID uuid.UUID `uri:"template_uuid" binding:"required"`
}

type TemplateResp struct {
	UUID uuid.UUID `json:"uuid"`
	Name string    `json:"name"`
}

type AllTemplateReq struct {
	LabUUID uuid.UUID `form:"lab_uuid" uri:"lab_uuid" binding:"required"`
}

type ResourceTemplates struct {
	Templates []*ResourceTemplate `json:"templates"`
}

type ResourceTemplate struct {
	UUID uuid.UUID `json:"uuid"`
	Name string    `json:"name"`
}

type ResourceReq struct {
	LabUUID uuid.UUID `json:"lab_uuid" form:"lab_uuid" uri:"lab_uuid"`
	Type    string    `json:"type" form:"type" uri:"type"`
}

type ResourceResp struct {
	ResourceNameList []*ResourceInfo `json:"resource_name_list"`
}

type ResourceInfo struct {
	UUID       uuid.UUID `json:"uuid"`
	Name       string    `json:"name"`
	ParentUUID uuid.UUID `json:"parent_uuid"`
}

type DeviceActionReq struct {
	LabUUID uuid.UUID `json:"lab_uuid" uri:"lab_uuid" form:"lab_uuid" binding:"required"`
	Name    string    `json:"name" uri:"name" form:"name" binding:"required"`
}

type DeviceActionResp struct {
	Name    string          `json:"name"`
	Actions []*DeviceAction `json:"actions"`
}

type DeviceAction struct {
	Action     string         `json:"action"`
	Schema     datatypes.JSON `json:"schema"`
	ActionType string         `json:"action_type"`
}

type StartMachineReq struct {
	LabUUID uuid.UUID `json:"lab_uuid" form:"lab_uuid" uri:"lab_uuid" binding:"required"`
}

type StartMachineRes struct {
	MachineUUID uuid.UUID `json:"machine_uuid"`
}

type DelMachineReq struct {
	LabUUID uuid.UUID `json:"lab_uuid" form:"lab_uuid" uri:"lab_uuid" binding:"required"`
}

type StopMachineReq struct {
	LabUUID uuid.UUID `json:"lab_uuid" form:"lab_uuid" uri:"lab_uuid" binding:"required"`
}

type MachineStatusReq struct {
	LabUUID uuid.UUID `json:"lab_uuid" form:"lab_uuid" uri:"lab_uuid" binding:"required"`
}

type MachineStatusRes struct {
	Status string `json:"status"`
}

type CreateMaterialReq struct {
	Nodes []*Material `json:"nodes"`
}

type CreateMaterialResp struct {
	UUID      uuid.UUID `json:"uuid"`
	CloudUUID uuid.UUID `json:"cloud_uuid"`
	DeviceID  string    `json:"id"`
	Name      string    `json:"name"`
}

type UpsertMaterialReq struct {
	MountUUID uuid.UUID   `json:"mount_uuid"`
	Nodes     []*Material `json:"nodes"`
}

type UpsertMaterialResp struct {
	UUID        uuid.UUID `json:"uuid"`
	CloudUUID   uuid.UUID `json:"cloud_uuid"`
	Name        string    `json:"name"`
	DisplayName string    `json:"display_name"`
}

type CreateMaterialEdgeReq struct {
	Edges []*MaterialEdge `json:"edges"`
}

type MaterialQueryReq struct {
	UUIDS        []uuid.UUID `json:"uuids"`
	WithChildren bool        `json:"with_children"`
}

type MaterialQueryResp struct {
	Nodes []*EdgeNode `json:"nodes"`
}

type EdgeNode struct {
	UUID        uuid.UUID      `json:"uuid"`
	ParentUUID  uuid.UUID      `json:"parent_uuid"`
	Name        string         `json:"name"`
	DisplayName string         `json:"display_name"`
	Description *string        `json:"description"`
	Class       string         `json:"class"`
	Status      string         `json:"status"`
	Type        string         `json:"type"`
	Config      datatypes.JSON `json:"config"`
	Schema      datatypes.JSON `json:"schema"`
	Data        datatypes.JSON `json:"data"`
	Pose        datatypes.JSON `json:"pose"`
	Model       datatypes.JSON `json:"model"`
	Icon        string         `json:"icon"`
}

type DownloadMaterialResp struct {
	Nodes []*Node `json:"nodes"`
}

type LabWS struct {
	LabUUID uuid.UUID `uri:"lab_uuid" binding:"required"`
}

// WSNode for websocket graph data
type WSNode struct {
	UUID        uuid.UUID      `json:"uuid"`
	ParentUUID  uuid.UUID      `json:"parent_uuid"`
	Name        string         `json:"name"`
	DisplayName string         `json:"display_name"`
	Type        string         `json:"type"`
	Data        datatypes.JSON `json:"data"`
	Status      string         `json:"status"`
	Icon        string         `json:"icon"`
}

type WSEdge struct {
	UUID             uuid.UUID `json:"uuid"`
	SourceNodeUUID   uuid.UUID `json:"source_node_uuid"`
	TargetNodeUUID   uuid.UUID `json:"target_node_uuid"`
	SourceHandleUUID uuid.UUID `json:"source_handle_uuid"`
	TargetHandleUUID uuid.UUID `json:"target_handle_uuid"`
	Type             string    `json:"type"`
}

type WSGraph struct {
	Nodes []*WSNode `json:"nodes"`
	Edges []*WSEdge `json:"edges"`
}

type UpdateNodes struct {
	Nodes []*WSNode `json:"nodes"`
}

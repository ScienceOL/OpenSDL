package model

type CreateMachineReq struct {
	UserID       string  `json:"-"`
	OrgID        string  `json:"-"`
	Name         string  `json:"name"`
	ImageID      uint64  `json:"imageId"`
	SkuID        int64   `json:"skuId"`
	ProjectID    uint64  `json:"projectId"`
	TurnoffAfter float32 `json:"turnoffAfter"`
	DiskSize     uint    `json:"diskSize"`
	Device       string  `json:"device"`
	Cmd          string  `json:"cmd"`
	Platform     string  `json:"platform"`
}

type RestartMachineReq struct {
	MachineID    int64   `json:"-"`
	UserID       string  `json:"-"`
	OrgID        string  `json:"-"`
	SkuID        int64   `json:"skuId"`
	DiskSize     uint    `json:"diskSize"`
	ProjectID    uint64  `json:"projectId"`
	Device       string  `json:"device"`
	TurnoffAfter float32 `json:"turnoffAfter"`
	Cmd          string  `json:"cmd"`
}

type JoninProjectReq struct {
	UserID    string `json:"-"`
	OrgID     string `json:"-"`
	ProjectID uint64 `json:"projectId"`
}

type NodeStatus int

const (
	NODE_STATUS_UNKNOW         NodeStatus = 0  // 状态未知
	NODE_STATUS_PENDING        NodeStatus = 1  // 启动中
	NODE_STATUS_RUNNING        NodeStatus = 2  // 运行中
	NODE_STATUS_STOPPING       NodeStatus = 3  // 停止中
	NODE_STATUS_IMAGE_BUILDING NodeStatus = 4  // 构建镜像中
	NODE_STATUS_STOPPED        NodeStatus = -1 // 已停止
	NODE_STATUS_FAIL           NodeStatus = -2 // 状态失败
	NODE_STATUS_DELETED        NodeStatus = -3 // 已经删除
)

type MachineStatusRes struct {
	Cost              int        `json:"cost"`
	Cpu               int        `json:"cpu"`
	Spec              string     `json:"spec"`
	CreateTime        string     `json:"createTime"`
	Creator           string     `json:"creator"`
	CreatorID         uint64     `json:"creatorId"`
	DiskSize          uint       `json:"diskSize"`
	UsedBytes         uint64     `json:"usedBytes"`
	InodeUsed         uint64     `json:"inodeUsed"`
	Gpu               string     `json:"gpu"`
	ImageName         string     `json:"imageName"`
	Ip                string     `json:"ip"`
	Kind              uint8      `json:"kind"`
	NodeID            uint64     `json:"nodeId"`
	NodeName          string     `json:"nodeName"`
	Memory            uint       `json:"memory"`
	ProjectID         uint64     `json:"projectId"`
	ProjectName       string     `json:"projectName"`
	ProjectRole       uint64     `json:"projectRole"`
	ReleaseType       int8       `json:"releaseType"`
	StopType          int8       `json:"stopType"`
	NodePwd           string     `json:"nodePwd"`
	NodeUser          string     `json:"nodeUser"`
	Status            NodeStatus `json:"status"`
	Device            string     `json:"device"`    // vm,container
	StartTime         string     `json:"startTime"` // 乐贝格开机时间
	EndTime           string     `json:"endTime"`
	EstimateStartTime string     `json:"estimateStartTime"` // 预计启动时间
	OperateStartTime  string     `json:"operateStartTime"`  // 点击启动时间
	UsedType          int8       `json:"usedType"`
	SkuID             uint64     `json:"skuId,omitempty"`
	IsAsk             bool       `json:"isAsk"`
	IsBeta            bool       `json:"isBeta"`
	StartingUpMsg     string     `json:"startingUpMsg"`
	MachineID         uint64     `json:"machineId"`
	IsNotebook        bool       `json:"isNotebook"`
	IsDomainBeta      bool       `json:"isDomainBeta"`
	DomainName        string     `json:"domainName"`
	Direct2webshell   bool       `json:"direct2webshell"`
	ReleaseTime       string     `json:"releaseTime"`
	HasConn           int8       `json:"hasConn"`
	ConnUrl           string     `json:"connUrl"`
}

type StopMachineReq struct {
	UserID    string `json:"-"`
	OrgID     string `json:"-"`
	MachineID int64  `json:"-"`
	CreatorID uint64 `json:"creatorId"`
	StopType  int8   `json:"stopType"` // 0: 直接关机 1: 保存镜像后关机
	Device    string `json:"device"`
	ProjectID uint64 `json:"projectID"`
}

type DelMachineReq struct {
	UserID    string `json:"-"`
	OrgID     string `json:"-"`
	MachineID int64  `json:"-"`
	CreatorID uint64 `json:"creatorId"`
	Device    string `json:"device"`
	ProjectID uint64 `json:"projectId"`
}

type MachineStatusReq struct {
	UserID    string `json:"-"`
	OrgID     string `json:"-"`
	MachineID int64  `json:"-"`
}

package reagent

import (
	// 外部依赖
	"time"

	// 内部引用
	common "github.com/scienceol/opensdl/service/pkg/common"
	uuid "github.com/scienceol/opensdl/service/pkg/common/uuid"
	repo "github.com/scienceol/opensdl/service/pkg/repo"
)

// InsertReq 与前端约定的字段名（下划线）
// 仅用于创建试剂的入参
// 注意：LabUUID 使用字符串以兼容现有前端传参
// 日期字段为可选字符串，格式需为 yyyy-mm-dd
// 其解析与校验在业务层完成
//
// 字段与原实现保持一致，避免前端改动
//
//nolint:revive
type InsertReq struct {
	LabUUID          uuid.UUID `json:"lab_uuid" binding:"required"`
	CAS              *string   `json:"cas"`
	Name             string    `json:"name" binding:"required"`
	MolecularFormula string    `json:"molecular_formula" binding:"required"`
	Smiles           *string   `json:"smiles"`
	StockInQuantity  *float64  `json:"stock_in_quantity" binding:"required"`
	Unit             string    `json:"unit" binding:"required"`
	Supplier         *string   `json:"supplier"`
	Color            *string   `json:"color"`
	ProductionDate   time.Time `json:"production_date" binding:"required"`
	ExpiryDate       time.Time `json:"expiry_date" binding:"required"`
}

// InsertResp 为创建试剂的返回结构
// 仅返回后端生成的资源 UUID
type InsertResp struct {
	UUID uuid.UUID `json:"uuid"`
}

// QueryReq 查询参数（分页采用 common.PageReq: page/page_size）
type QueryReq struct {
	common.PageReq

	LabUUID        uuid.UUID         `form:"lab_uuid"`
	CAS            *string           `form:"cas"`
	Name           *string           `form:"name"`
	Supplier       *string           `form:"supplier"`
	StockStatus    *repo.StockStatus `form:"stock_status"` // 1=充足, 2=不足
	ValidStatus    *repo.ValidStatus `form:"valid_status"` // 1=有效, 2=失效
	CreatedDate    *repo.SortOrder   `form:"created_date"`
	ProductionDate *repo.SortOrder   `form:"production_date"`
	ExpiryDate     *repo.SortOrder   `form:"expiry_date"`
}

// ReagentResponse 查询返回的单条数据
type ReagentResponse struct {
	UUID             uuid.UUID        `json:"uuid"`
	CreatedAt        time.Time        `json:"created_at"`
	UpdatedAt        time.Time        `json:"updated_at"`
	CAS              *string          `json:"cas"`
	Name             string           `json:"name"`
	MolecularFormula string           `json:"molecular_formula"`
	Smiles           *string          `json:"smiles"`
	StockInQuantity  float64          `json:"stock_in_quantity"`
	Unit             string           `json:"unit"`
	Supplier         *string          `json:"supplier"`
	Color            *string          `json:"color"`
	ProductionDate   *time.Time       `json:"production_date"`
	ExpiryDate       *time.Time       `json:"expiry_date"`
	StockStatus      repo.StockStatus `json:"stock_status"` // 1=充足, 2=不足
	ValidStatus      repo.ValidStatus `json:"valid_status"` // 1=有效, 2=失效
}

// QueryResp 查询返回
type QueryResp struct {
	Total int64              `json:"total"`
	List  []*ReagentResponse `json:"list"`
}

// DeleteReq 删除参数
type DeleteReq struct {
	UUID uuid.UUID `json:"uuid" binding:"required"`
}

// UpdateReq 更新参数
type UpdateReq struct {
	ReagentUpdateData []*ReagentUpdateData `json:"reagent_data"`
	LabUUID           uuid.UUID            `json:"lab_uuid" binding:"required"`
}

type ReagentUpdateData struct {
	UUID                uuid.UUID  `json:"uuid" binding:"required"`
	CAS                 *string    `json:"cas"`
	Name                *string    `json:"name"`
	MolecularFormula    *string    `json:"molecular_formula"`
	Smiles              *string    `json:"smiles"`
	StockInQuantity     *float64   `json:"stock_in_quantity"`
	ConsumptionQuantity *float64   `json:"consumption_quantity" binding:"omitempty,gte=0"`
	Unit                *string    `json:"unit"`
	Supplier            *string    `json:"supplier"`
	Color               *string    `json:"color"`
	ProductionDate      *time.Time `json:"production_date"`
	ExpiryDate          *time.Time `json:"expiry_date"`
}

// CAS 查询
type CasReq struct {
	CAS string `form:"cas" json:"cas" binding:"required"`
}

type CasResp struct {
	Name             string `json:"name"`
	MolecularFormula string `json:"molecular_formula"`
	SMILES           string `json:"smiles"`
}

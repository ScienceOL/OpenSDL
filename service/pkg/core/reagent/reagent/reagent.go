package reagent

import (
	// 外部依赖
	"context"
	"strings"
	"time"

	// 内部引用
	common "github.com/scienceol/opensdl/service/pkg/common"
	code "github.com/scienceol/opensdl/service/pkg/common/code"
	core "github.com/scienceol/opensdl/service/pkg/core/reagent"
	auth "github.com/scienceol/opensdl/service/pkg/middleware/auth"
	logger "github.com/scienceol/opensdl/service/pkg/middleware/logger"
	repo "github.com/scienceol/opensdl/service/pkg/repo"
	model "github.com/scienceol/opensdl/service/pkg/model"
	repoPubchem "github.com/scienceol/opensdl/service/pkg/repo/pubchem"
	repoReagent "github.com/scienceol/opensdl/service/pkg/repo/reagent"
	utils "github.com/scienceol/opensdl/service/pkg/utils"
)

type reagentImpl struct {
	reagentStore repo.ReagentRepo
	pubchem      repo.PubChemRepo
}

func New() core.Service {
	return &reagentImpl{reagentStore: repoReagent.NewReagentRepo(), pubchem: repoPubchem.NewPubChemRepo()}
}

// Insert 业务实现：
// - 解析/校验参数
// - 从 ctx 中获取当前用户
// - lab_uuid -> lab_id 映射
// - 构造 model.Reagent 写库
// - 返回生成的资源 uuid
func (r *reagentImpl) Insert(ctx context.Context, req *core.InsertReq) (*core.InsertResp, error) {
	currentUser := auth.GetCurrentUser(ctx)
	if currentUser == nil {
		return nil, code.UnLogin
	}

	labID := r.reagentStore.UUID2ID(ctx, &model.Laboratory{}, req.LabUUID)[req.LabUUID]
	if labID == 0 {
		return nil, code.LabNotFound
	}

	data := &model.Reagent{
		LabID:            labID,
		UserID:           currentUser.ID,
		CAS:              req.CAS,
		Name:             req.Name,
		MolecularFormula: req.MolecularFormula,
		Smiles:           req.Smiles,
		StockInQuantity:  *req.StockInQuantity,
		Unit:             req.Unit,
		Supplier:         req.Supplier,
		Color:            req.Color,
		ProductionDate:   &req.ProductionDate,
		ExpiryDate:       &req.ExpiryDate,
	}

	if err := r.reagentStore.CreateData(ctx, data); err != nil {
		logger.Errorf(ctx, "CreateReagent err: %+v", err)
		return nil, code.ReagentCreateErr.WithErr(err)
	}

	return &core.InsertResp{UUID: data.UUID}, nil
}

// Query 查询列表
func (r *reagentImpl) Query(ctx context.Context, req *core.QueryReq) (*common.PageResp[[]*core.ReagentResponse], error) {
	repoImpl := r.reagentStore
	q := repo.ReagentQuery{}

	if req.LabUUID.IsNil() {
		return nil, code.LabNotFound
	}

	if q.LabID = r.reagentStore.UUID2ID(ctx, &model.Laboratory{}, req.LabUUID)[req.LabUUID]; q.LabID == 0 {
		return nil, code.LabNotFound
	}

	if req.CAS != nil && *req.CAS != "" {
		q.CAS = req.CAS
	}
	if req.Name != nil && *req.Name != "" {
		q.NameLike = req.Name
	}
	if req.Supplier != nil && *req.Supplier != "" {
		q.Supplier = req.Supplier
	}
	if req.StockStatus != nil {
		q.Stock = req.StockStatus
	}
	if req.ValidStatus != nil {
		q.Valid = req.ValidStatus
	}

	orderClauses := make([]string, 0, 3)
	if req.CreatedDate != nil {
		orderClauses = append(orderClauses, "created_at "+string(*req.CreatedDate))
	}
	if req.ProductionDate != nil {
		orderClauses = append(orderClauses, "production_date "+string(*req.ProductionDate))
	}
	if req.ExpiryDate != nil {
		orderClauses = append(orderClauses, "expiry_date "+string(*req.ExpiryDate))
	}
	if len(orderClauses) > 0 {
		q.OrderBy = strings.Join(orderClauses, ",")
	} else {
		q.OrderBy = "id desc"
	}

	req.Normalize()
	q.Offset = req.PageReq.Offest()
	q.Limit = req.PageSize

	list, total, err := repoImpl.ListReagents(ctx, q)
	if err != nil {
		return nil, code.ReagentQueryErr.WithErr(err)
	}

	today := time.Now()
	respList := utils.FilterSlice(list, func(r *model.Reagent) (*core.ReagentResponse, bool) {
		stockStatus := repo.StockInsufficient
		if r.StockInQuantity > 0 {
			stockStatus = repo.StockSufficient
		}
		validStatus := repo.ValidValid
		if r.ExpiryDate != nil &&
			!r.ExpiryDate.IsZero() &&
			r.ExpiryDate.Before(today) {
			validStatus = repo.ValidExpired
		}
		return &core.ReagentResponse{
			UUID:             r.UUID,
			CreatedAt:        r.CreatedAt,
			UpdatedAt:        r.UpdatedAt,
			CAS:              r.CAS,
			Name:             r.Name,
			MolecularFormula: r.MolecularFormula,
			Smiles:           r.Smiles,
			StockInQuantity:  r.StockInQuantity,
			Unit:             r.Unit,
			Supplier:         r.Supplier,
			Color:            r.Color,
			ProductionDate:   r.ProductionDate,
			ExpiryDate:       r.ExpiryDate,
			StockStatus:      stockStatus,
			ValidStatus:      validStatus,
		}, true
	})

	return &common.PageResp[[]*core.ReagentResponse]{
		Data:     respList,
		Total:    total,
		Page:     req.Page,
		PageSize: req.PageSize,
	}, nil
}

// Delete 删除
func (r *reagentImpl) Delete(ctx context.Context, req *core.DeleteReq) error {
	return r.reagentStore.UpdateData(ctx, &model.Reagent{IsDeleted: 1}, map[string]any{
		"uuid": req.UUID,
	}, "is_deleted")
}

// Update 更新
func (r *reagentImpl) Update(ctx context.Context, req *core.UpdateReq) error {
	if len(req.ReagentUpdateData) == 0 {
		return code.ParamErr.WithMsg("empty update request")
	}

	return r.reagentStore.ExecTx(ctx, func(txCtx context.Context) error {
		for _, req := range req.ReagentUpdateData {
			if err := r.updateOne(txCtx, req); err != nil {
				return code.ReagentConsumeErr.WithErr(err)
			}
		}
		return nil
	})
}

func (r *reagentImpl) updateOne(ctx context.Context, req *core.ReagentUpdateData) error {
	updateData := make(map[string]any)
	if req.CAS != nil {
		updateData["cas"] = *req.CAS
	}
	if req.Name != nil {
		updateData["name"] = *req.Name
	}
	if req.MolecularFormula != nil {
		updateData["molecular_formula"] = *req.MolecularFormula
	}
	if req.Smiles != nil {
		updateData["smiles"] = *req.Smiles
	}
	if req.StockInQuantity != nil {
		updateData["stock_in_quantity"] = *req.StockInQuantity
	} else if req.ConsumptionQuantity != nil {
		if err := r.reagentStore.ConsumeStock(ctx, req.UUID, *req.ConsumptionQuantity); err != nil {
			return err
		}
		return nil
	}
	if req.Unit != nil {
		updateData["unit"] = *req.Unit
	}
	if req.Supplier != nil {
		updateData["supplier"] = *req.Supplier
	}
	if req.Color != nil {
		updateData["color"] = *req.Color
	}
	if req.ProductionDate != nil {
		updateData["production_date"] = *req.ProductionDate
	}
	if req.ExpiryDate != nil {
		updateData["expiry_date"] = *req.ExpiryDate
	}

	if len(updateData) == 0 {
		return code.ParamErr.WithMsg("at least one field to update is required")
	}

	return r.reagentStore.UpdateReagentByUUID(ctx, req.UUID, updateData)
}

// QueryCAS 通过 CAS 编号获取化合物信息，使用 repo/pubchem 的 client
func (r *reagentImpl) QueryCAS(ctx context.Context, req *core.CasReq) (*core.CasResp, error) {
	if req.CAS == "" {
		return nil, code.ParamErr.WithMsg("cas is required")
	}

	info, err := r.pubchem.GetCompoundByCAS(ctx, req.CAS)
	if err != nil {
		return nil, code.ReagentCASQueryErr.WithErr(err)
	}
	if info == nil {
		return nil, code.ReagentCASNotFindErr.WithMsg("cas not found")
	}
	return &core.CasResp{
		Name:             info.Name,
		MolecularFormula: info.MolecularFormula,
		SMILES:           info.SMILES,
	}, nil
}

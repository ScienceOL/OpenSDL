package pubchem

import (
	// 外部依赖
	"context"
	"net/http"
	"time"

	resty "github.com/go-resty/resty/v2"

	// 内部引用
	config "github.com/scienceol/opensdl/service/internal/config"
	code "github.com/scienceol/opensdl/service/pkg/common/code"
	logger "github.com/scienceol/opensdl/service/pkg/middleware/logger"
	repo "github.com/scienceol/opensdl/service/pkg/repo"
)

type property struct {
	Title            string `json:"Title"`
	MolecularFormula string `json:"MolecularFormula"`
	IUPACName        string `json:"IUPACName"`
	IsomericSMILES   string `json:"IsomericSMILES"`
	CanonicalSMILES  string `json:"CanonicalSMILES"`
	SMILES           string `json:"SMILES"`
}

type PropertyResponse struct {
	PropertyTable struct {
		Properties []property `json:"Properties"`
	} `json:"PropertyTable"`
}

type pubchemImpl struct {
	client *resty.Client
}

func NewPubChemRepo() repo.PubChemRepo {
	baseURL := config.Global().RPC.PubChem.Addr

	return &pubchemImpl{
		client: resty.New().
			SetTimeout(30*time.Second).
			EnableTrace().
			SetBaseURL(baseURL).
			SetHeader("Content-Type", "application/json"),
	}
}

func (p *pubchemImpl) GetCompoundByCAS(ctx context.Context, cas string) (*repo.CompoundInfo, error) {
	properties := "Title,MolecularFormula,IUPACName,IsomericSMILES,CanonicalSMILES,SMILES"
	urlPath := "/rest/pug/compound/name/{cas}/property/{props}/JSON"

	propResp := &PropertyResponse{}
	res, err := p.client.R().
		SetContext(ctx).
		SetPathParams(map[string]string{
			"props": properties,
			"cas":   cas,
		}).
		SetResult(propResp).
		Get(urlPath)
	if err != nil {
		logger.Errorf(ctx, "Failed to request properties from PubChem: %v", err)
		return nil, code.RPCHttpErr.WithErr(err)
	}

	if res.StatusCode() != http.StatusOK {
		return nil, code.RPCHttpCodeErr.WithMsgf("PubChem property query failed: status %d", res.StatusCode())
	}

	if len(propResp.PropertyTable.Properties) == 0 {
		return nil, code.UnDefineErr.WithMsg("Failed to parse PubChem property response")
	}

	propData := propResp.PropertyTable.Properties[0]

	name := propData.Title
	if name == "" {
		name = propData.IUPACName
	}

	smiles := propData.IsomericSMILES
	if smiles == "" {
		smiles = propData.CanonicalSMILES
	}
	if smiles == "" {
		smiles = propData.SMILES
	}

	return &repo.CompoundInfo{
		Name:             name,
		MolecularFormula: propData.MolecularFormula,
		SMILES:           smiles,
	}, nil
}

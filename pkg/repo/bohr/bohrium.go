package bohr

import (
	"context"
	"errors"
	"net/http"
	"strconv"

	"github.com/go-resty/resty/v2"
	"github.com/scienceol/osdl/internal/config"
	"github.com/scienceol/osdl/pkg/common"
	"github.com/scienceol/osdl/pkg/common/code"
	"github.com/scienceol/osdl/pkg/middleware/db"
	"github.com/scienceol/osdl/pkg/middleware/logger"
	"github.com/scienceol/osdl/pkg/repo"
	"github.com/scienceol/osdl/pkg/repo/model"
	"gorm.io/gorm"
)

type AccessKeyUserInfo struct {
	UserID int64 `json:"userId"`
	OrgID  int64 `json:"orgId"`
}

type BohrImpl struct {
	*db.Datastore
	bohrCore *resty.Client
}

func New() repo.Account {
	conf := config.Global().RPC.BohrCore
	return &BohrImpl{
		Datastore: db.DB(),
		bohrCore: resty.New().
			EnableTrace().
			SetBaseURL(conf.Addr),
	}
}

func (b *BohrImpl) GetLabUserInfo(ctx context.Context, req *model.LabAkSk) (*model.UserData, error) {
	labData := &model.Laboratory{}
	if err := b.DBWithContext(ctx).
		Where("access_key = ? AND access_secret = ?",
			req.AccessKey, req.AccessSecret).
		Select("id", "user_id", "uuid").
		Take(labData).Error; err != nil {
		if errors.Is(err, gorm.ErrRecordNotFound) {
			return nil, code.RecordNotFound
		}
		logger.Errorf(ctx, "GetLabUserInfo fail err: %+v", err)
		return nil, err
	}

	return &model.UserData{
		ID:      labData.UserID,
		LabID:   labData.ID,
		LabUUID: labData.UUID,
	}, nil
}

func (b *BohrImpl) GetLabUserByAccessKey(ctx context.Context, accessKey string) (*model.UserData, error) {
	respData := &common.RespT[*AccessKeyUserInfo]{}
	resp, err := b.bohrCore.R().
		SetContext(ctx).
		SetQueryParam("accessKey", accessKey).
		SetResult(respData).
		Get("/api/v1/ak/get_user")
	if err != nil {
		logger.Errorf(ctx, "GetLabUserByAccessKey err: %+v", err)
		return nil, code.RPCHttpErr.WithMsg(err.Error())
	}

	if resp.StatusCode() != http.StatusOK {
		logger.Errorf(ctx, "GetLabUserByAccessKey http code: %d", resp.StatusCode())
		return nil, code.RPCHttpCodeErr.WithMsgf("GetLabUserByAccessKey code: %d", resp.StatusCode())
	}

	if respData.Code != code.Success {
		logger.Errorf(ctx, "GetLabUserByAccessKey resp code not zero err msg: %+v", respData.Error)
		return nil, code.RPCHttpCodeErr.WithMsgf("GetLabUserByAccessKey resp code: %d", respData.Code)
	}

	return &model.UserData{
		Owner: strconv.FormatInt(respData.Data.OrgID, 10),
		ID:    strconv.FormatInt(respData.Data.UserID, 10),
	}, nil
}

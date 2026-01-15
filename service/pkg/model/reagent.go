package model

import (
	"time"
)

type Reagent struct {
	BaseModel
	LabID            int64      `gorm:"not null;index:idx_reagent_lab_id" json:"lab_id"`
	UserID           string     `gorm:"type:varchar(120);not null;index:idx_reagent_user_id" json:"user_id"`
	CAS              *string    `gorm:"type:varchar(64);index:idx_reagent_cas" json:"cas"`
	Name             string     `gorm:"type:varchar(255);not null;index:idx_reagent_name" json:"name"`
	MolecularFormula string     `gorm:"type:text;not null" json:"molecular_formula"`
	Smiles           *string    `gorm:"type:text" json:"smiles"`
	StockInQuantity  float64    `gorm:"type:numeric(12,3);not null;default:0;check:stock_in_quantity >= 0" json:"stock_in_quantity"`
	Unit             string     `gorm:"type:varchar(32);not null" json:"unit"`
	Supplier         *string    `gorm:"type:varchar(255);index:idx_reagent_supplier" json:"supplier"`
	Color            *string    `gorm:"type:varchar(32)" json:"color"`
	ProductionDate   *time.Time `gorm:"type:date" json:"production_date"`
	ExpiryDate       *time.Time `gorm:"type:date;index:idx_reagent_expiry_date" json:"expiry_date"`
	IsDeleted        int8       `gorm:"type:smallint;not null;default:0" json:"is_deleted"`
}

func (*Reagent) TableName() string { return "reagent" }
